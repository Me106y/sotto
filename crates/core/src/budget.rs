//! 延迟预算模型。
//!
//! 没有真实 API key 时，这套模型能回答一个比「首字几毫秒」更重要的问题：
//! **哪一阶段一旦变慢，整个 1.5s 目标就守不住。**
//!
//! 每个阶段用三角分布采样（min / mode / max），独立求和 —— 这比「把各阶段
//! P90 相加」诚实得多，因为多个阶段同时打满 P90 的概率其实很低。

use crate::types::Stage;

#[derive(Debug, Clone, Copy)]
pub struct StageModel {
    pub stage: Stage,
    pub min_ms: f32,
    pub mode_ms: f32,
    pub max_ms: f32,
}

impl StageModel {
    pub fn new(stage: Stage, min_ms: f32, mode_ms: f32, max_ms: f32) -> Self {
        Self {
            stage,
            min_ms,
            mode_ms,
            max_ms,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Budget {
    pub stages: Vec<StageModel>,
}

#[derive(Debug, Clone)]
pub struct BudgetResult {
    pub p50: f32,
    pub p90: f32,
    pub p99: f32,
    pub mean: f32,
    /// 落在 1500ms 以内的比例（0~100）
    pub under_1500: f32,
    /// 落在 2500ms 以内的比例（0~100）
    pub under_2500: f32,
    pub stage_mean: Vec<(Stage, f32)>,
}

impl Budget {
    /// PRD §6 的预算表
    pub fn prd_default() -> Self {
        Self {
            stages: vec![
                StageModel::new(Stage::Vad, 300.0, 400.0, 500.0),
                StageModel::new(Stage::AsrFinalize, 100.0, 150.0, 200.0),
                StageModel::new(Stage::Retrieve, 50.0, 100.0, 150.0),
                StageModel::new(Stage::FirstToken, 250.0, 400.0, 600.0),
                StageModel::new(Stage::TtsFirstPacket, 200.0, 300.0, 400.0),
            ],
        }
    }

    /// 把某个阶段的分布整体乘一个系数，用来看敏感性。
    pub fn scaled(&self, stage: Stage, factor: f32) -> Self {
        let mut stages = self.stages.clone();
        for s in stages.iter_mut() {
            if s.stage == stage {
                s.min_ms *= factor;
                s.mode_ms *= factor;
                s.max_ms *= factor;
            }
        }
        Self { stages }
    }

    pub fn simulate(&self, trials: usize, seed: u64) -> BudgetResult {
        let mut rng = Lcg::new(seed);
        let mut totals: Vec<f32> = Vec::with_capacity(trials);
        let mut stage_sum: Vec<f32> = vec![0.0; self.stages.len()];

        for _ in 0..trials {
            let mut total = 0.0;
            for (i, s) in self.stages.iter().enumerate() {
                let u = rng.next_f32();
                let v = triangular(s.min_ms, s.mode_ms, s.max_ms, u);
                stage_sum[i] += v;
                total += v;
            }
            totals.push(total);
        }

        totals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = totals.len();
        let mean = totals.iter().sum::<f32>() / n as f32;
        let under_1500 = totals.iter().filter(|v| **v <= 1500.0).count() as f32 / n as f32 * 100.0;
        let under_2500 = totals.iter().filter(|v| **v <= 2500.0).count() as f32 / n as f32 * 100.0;

        BudgetResult {
            p50: percentile(&totals, 0.50),
            p90: percentile(&totals, 0.90),
            p99: percentile(&totals, 0.99),
            mean,
            under_1500,
            under_2500,
            stage_mean: self
                .stages
                .iter()
                .enumerate()
                .map(|(i, s)| (s.stage, stage_sum[i] / trials as f32))
                .collect(),
        }
    }
}

fn percentile(sorted: &[f32], q: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f32 * q).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// 三角分布采样：a ≤ mode ≤ b
fn triangular(a: f32, m: f32, b: f32, u: f32) -> f32 {
    if (b - a).abs() < f32::EPSILON {
        return a;
    }
    let c = (m - a) / (b - a);
    if u < c {
        a + ((u * (b - a) * (m - a)).max(0.0)).sqrt()
    } else {
        b - (((1.0 - u) * (b - a) * (b - m)).max(0.0)).sqrt()
    }
}

/// 极小的 LCG，只为了让结果可复现——不需要密码学强度。
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(2).wrapping_add(1))
    }
    fn next_f32(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        // 取高 24 位
        ((self.0 >> 40) as f32) / (1u32 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prd_budget_is_bound_to_the_product_target() {
        let r = Budget::prd_default().simulate(20_000, 42);
        assert!(r.p50 < r.p90 && r.p90 < r.p99, "p50={} p90={}", r.p50, r.p90);
        // PRD 假设的分布下，P50 应该落在 1.3~1.5s 附近
        assert!(r.p50 > 1000.0 && r.p50 < 1600.0, "p50={}", r.p50);
        assert_eq!(r.stage_mean.len(), 5);
    }

    #[test]
    fn simulation_is_deterministic_for_a_seed() {
        let a = Budget::prd_default().simulate(5_000, 7);
        let b = Budget::prd_default().simulate(5_000, 7);
        assert_eq!(a.p50, b.p50);
        assert_eq!(a.p90, b.p90);
    }

    #[test]
    fn slowing_one_stage_moves_the_total() {
        let base = Budget::prd_default().simulate(20_000, 1);
        let slow = Budget::prd_default()
            .scaled(Stage::FirstToken, 2.5)
            .simulate(20_000, 1);
        assert!(slow.p50 > base.p50 + 300.0);
    }

    #[test]
    fn triangular_respects_bounds() {
        for u in [0.0, 0.25, 0.5, 0.75, 0.999] {
            let v = triangular(100.0, 200.0, 400.0, u);
            assert!(v >= 100.0 && v <= 400.0, "u={u} v={v}");
        }
    }
}
