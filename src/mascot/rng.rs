//! Java `java.util.Random` 逐語相当の乱数実装体（48 ビット LCG）。
//!
//! Java 正本（pin 値は JDK 17.0.19 実機照合・tests/rng_test.rs 参照）:
//! - `Random(long seed)`: `initialScramble` — `seed = (seed ^ 0x5DEECE66D)
//!   & ((1L << 48) - 1)`。負 seed も `& mask` で正規化されるため構築時に
//!   負数の特別扱いは不要
//! - `next(bits)`: `seed = (seed * 0x5DEECE66D + 0xB) & ((1L << 48) - 1)`
//!   → `(int)(seed >>> (48 - bits))`
//! - `nextDouble()`: `(((long)(next(26)) << 27) + next(27)) / (1L << 53)`
//!   （`n / 2^53`（n < 2^53）は f64 に厳密表現されるため bit-exact）
//!
//! Java の `Math.random()` = `nextDouble()` 1 呼び出し相当であるため、
//! [`crate::mascot::Rng::unit`]（[0,1) 一様乱数 1 値）を `nextDouble()` 逐語で
//! 実装する（design.md §1.5(e): 行動選択・Dragged/Regist の乱数を
//! `&mut dyn Rng` として注入。所有は Manager = #10 統合・`Box<dyn Rng>`）。
//!
//! シード供給経路:
//! - [`JavaRandom::new`]: 明示 seed（テスト・再現が必要な経路）
//! - [`JavaRandom::from_os`]: OS 由来（UNIX_EPOCH 以降 nanos ^ プロセス ID ^
//!   静的カウンタ）。依存クレート追加禁止（AGENTS.md §3）のため std のみで混合する
//!
//! Java は整数演算が wrapping 既定（オーバーフローで 2 の補数 wrap・パニックなし）
//! のため i64 演算は `wrapping_*` を使用する（`seed * 0x5DEECE66D` は 74 ビットに
//! 膨らむため必須・wrap 後の `& mask` で下位 48 ビットが Java と一致する）。
//! `>>>`（論理右シフト）は `u64` 化してから適用する。

use std::sync::atomic::{AtomicU64, Ordering};

use crate::mascot::Rng;

/// 48 ビット mask（Java `Random.mask` = `(1L << 48) - 1`）。
const MASK: i64 = (1i64 << 48) - 1;
/// Java `Random.multiplier` = `0x5DEECE66D`。
const MULTIPLIER: i64 = 0x5DEECE66D;
/// Java `Random.addend` = `0xB`。
const ADDEND: i64 = 0xB;
/// Java `seedUniquifier()` の更新係数（L'Ecuyer 1999）逐語 = `1181783497276652981`
/// （= `0x1066_89D4_5497_FDB5`）。
const UNIQUIFIER_STEP: u64 = 1181783497276652981;

/// Java static `Random.seedUniquifier`（AtomicLong）逐語相当の静的カウンタ
/// （エントロピ用途）。グローバル可変状態禁止（AGENTS.md §5-3）の対象は
/// マスコット/アプリ状態であり、シード混合専用の静的カウンタは Java 自身も
/// static で持ちマスコット状態には一切関与しないため例外とする
/// （判断記録: design.md §1.10(f)）。
static SEED_UNIQUIFIER: AtomicU64 = AtomicU64::new(0);

/// Java `java.util.Random` 逐語相当の乱数生成器（`Math.random()` の正本）。
pub struct JavaRandom {
    /// 内部状態（Java `Random.seed`。常に 48 ビット mask 適用済み）。
    seed: i64,
}

impl JavaRandom {
    /// Java `Random(long seed)` 相当（構築時に `initialScramble` を適用）。
    /// 負 seed も scramble の `& mask` で正規化されるため分岐は不要。
    pub fn new(seed: i64) -> Self {
        JavaRandom {
            seed: (seed ^ MULTIPLIER) & MASK,
        }
    }

    /// OS 由来シードで構築する（Java `new Random()` 逐語相当）。
    /// シード = UNIX_EPOCH 以降の経過 nanos ^ プロセス ID ^ 静的カウンタ
    /// （Java `seedUniquifier` 相当。時計量子以下の連続構築でも構造的に
    /// 同 seed にならない・std のみで依存追加なし）。
    pub fn from_os() -> Self {
        let uniq = SEED_UNIQUIFIER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let pid = u64::from(std::process::id());
        JavaRandom::new((nanos ^ pid ^ uniq.wrapping_mul(UNIQUIFIER_STEP)) as i64)
    }

    /// Java `next(bits)` 逐語。
    fn next(&mut self, bits: u32) -> i32 {
        // seed 更新は wrapping（乗算が 74 ビットに膨らむ。Java は wrap 後 mask で
        // 下位 48 ビットが本命のため wrapping が逐語一致）
        self.seed = (self.seed.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND)) & MASK;
        // Java `(int)(seed >>> (48 - bits))` — 論理シフトのため u64 化してから右シフト
        ((self.seed as u64) >> (48 - bits)) as i32
    }
}

impl Rng for JavaRandom {
    /// Java `nextDouble()` 逐語（= `Math.random()` 相当・[0,1) 半開区間）。
    fn unit(&mut self) -> f64 {
        // ((long)(next(26)) << 27) + next(27) — 最大 2^53 - 1 で i64 に収まる
        let hi = i64::from(self.next(26));
        let lo = i64::from(self.next(27));
        let n = hi.wrapping_shl(27).wrapping_add(lo);
        // n / 2^53 — n は 53 ビット整数のため変換・除算とも bit-exact
        n as f64 / (1i64 << 53) as f64
    }
}
