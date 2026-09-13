//! タスク #10b-2a: `src/mascot/rng.rs`（Java `java.util.Random` 逐語相当の
//! OS 乱数実装体 `JavaRandom`）の契約テスト（TDD RED）。
//!
//! Java 正本: `java.util.Random.nextDouble()`
//! = `(((long)(next(26)) << 27) + next(27)) / (1L << 53)`、
//! `next(bits)` = seed scramble（`seed = (seed ^ 0x5DEECE66D) & ((1 << 48) - 1)`
//! → `seed = (seed * 0x5DEECE66D + 0xB) & mask` → `(int)(seed >>> (48 - bits))`）。
//! 負 seed は初回 scramble の `& mask` で正規化されるため同一経路。
//!
//! pin 値の正本は **javac 17.0.19 実機照合**（JDK: OpenJDK 17.0.19+10-LTS）。
//! 証跡: `C:\Users\LEADSM~1\AppData\Local\Temp\opencode\rng-ref\RngRef.java`
//! （`java.util.Random(seed)` から `nextDouble()` を 10 回連続呼び出しし、
//! `Double.doubleToLongBits` の 16 進 bits を出力。実行出力は同配下 RngRef 実行ログ）。
//! 期待値は f64 のビット列（`f64::from_bits`）で焼き込み、**ビット完全一致**を要求する
//! （nextDouble の結果は n / 2^53（n < 2^53）の厳密表現のため bit-exact が成立する）。
//!
//! 契約:
//! - `shimeji::mascot::rng::JavaRandom::new(seed: i64)` — seed 指定構築。
//!   連続 `unit()`（= `shimeji::mascot::Rng::unit`）が JDK nextDouble 列と
//!   呼び出し順・値とも完全一致（seed 42 / 0 / -42 / 987654321098765432 × 各 10 値）。
//!   seed 0（scramble のみで立つ経路）と負 seed（内部マスク正規化経路）を含む。
//! - 全 pin 値が [0, 1) に収まる。
//! - `Box<dyn Rng>` として構築でき、trait object 経由の連続呼び出しで
//!   同一列になる（Manager への `Box<dyn Rng>` 注入契約）。
//! - `JavaRandom::from_os()` — OS シード構築。連続 `unit()` が
//!   全部同一にならず、各値が [0, 1) に収まる（シード品質の smoke pin。
//!   分布品質の統計テストは過剰のため行わない）。
//! - 同プロセス内で `from_os()` を**連続 5 回構築**した際、各インスタンスの
//!   先頭 `unit()` が 5 値すべて異なる（seed 衝突がないことの pin）。
//!   seed = pid ^ 時計 nanos のみでは pid がプロセス内一定のため、
//!   時計量子（~1.4µs）以下の間隔の連続構築で同 seed → 同一乱数列に
//!   なる（verifier 3/3 再現）。Java `new Random()` の static
//!   seedUniquifier（係数 1181783497276652981L・JDK 17.0.19 Random.java L117
//!   実物）相当の uniq カウンタ混合を
//!   実装に要求する pin であり、修正前実装では FAIL になることが正常
//!   （RED・偶発通過の可能性は実質ゼロに近い）。
//!
//! TDD RED: `src/mascot/rng.rs` は未作成のため import 未解決（E0432）の
//! コンパイルエラーになることが正常。

use shimeji::mascot::rng::JavaRandom;
use shimeji::mascot::Rng;

/// seed 42 の `new Random(42).nextDouble()` × 10 値（javac 17.0.19 実行出力の
/// `Double.doubleToLongBits` 16 進・RngRef.java SEED 42 ブロック）。
const PINS_SEED_42: [u64; 10] = [
    0x3fe7_4833_a06f_f457, // 0.7275636800328681
    0x3fe5_dcf7_7862_2e01, // 0.6832234717598454
    0x3fd3_c20f_3f12_bbb4, // 0.30871945533265976
    0x3fd1_bba7_6b52_c856, // 0.27707849007413665
    0x3fe5_4c2d_50bb_0864, // 0.6655489517945736
    0x3fec_e86c_f39c_2cbe, // 0.9033722646721782
    0x3fd7_9a23_a61b_35c8, // 0.36878291341130565
    0x3fd1_a5db_3b0b_fbe2, // 0.2757480694417024
    0x3fdd_ac80_0c31_8574, // 0.46365357580915334
    0x3fe9_0d88_07fc_450f, // 0.7829017787900358
];

/// seed 0 の `new Random(0).nextDouble()` × 10 値（RngRef.java SEED 0 ブロック）。
const PINS_SEED_0: [u64; 10] = [
    0x3fe7_6416_8ea6_ca89, // 0.730967787376657
    0x3fce_c9e5_b367_2e14, // 0.24053641567148587
    0x3fe4_65b9_3a78_ef81, // 0.6374174253501083
    0x3fe1_9d2e_10ef_a128, // 0.5504370051176339
    0x3fe3_1f17_4640_953b, // 0.5975452777972018
    0x3fd5_5373_440b_5f04, // 0.3332183994766498
    0x3fd8_a6f0_89ce_fe94, // 0.3851891847407185
    0x3fef_83d2_67dc_d07a, // 0.984841540199809
    0x3fec_2243_602f_4588, // 0.8791825178724801
    0x3fee_1eb6_9968_6687, // 0.9412491794821144
];

/// seed -42 の `new Random(-42).nextDouble()` × 10 値（RngRef.java SEED -42
/// ブロック・負 seed は scramble の `& ((1 << 48) - 1)` で正規化される経路）。
const PINS_SEED_NEGATIVE_42: [u64; 10] = [
    0x3fd1_7288_268c_4062, // 0.2726154686397476
    0x3faf_34cd_da01_0ad0, // 0.06094973837072859
    0x3fd1_e9b8_9c94_5960, // 0.2798902062508173
    0x3fd5_f300_d2ed_9420, // 0.34295673941079663
    0x3fe4_b891_97ac_4d86, // 0.647530361401025
    0x3fcb_ccc8_cd2a_3d88, // 0.21718702333280882
    0x3fd5_9c09_ba95_0f4a, // 0.3376488039104869
    0x3fed_a502_aa26_5d36, // 0.9263928721656274
    0x3fed_e6cd_1400_0121, // 0.9344239607453667
    0x3fe7_5d67_5fce_d642, // 0.7301518317460209
];

/// seed 987654321098765432 の `nextDouble()` × 10 値（RngRef.java
/// SEED 987654321098765432 ブロック・48 ビット mask を超える大きい正 seed）。
const PINS_SEED_LARGE: [u64; 10] = [
    0x3feb_942e_dd22_2714, // 0.8618387526523485
    0x3fea_07ad_4941_cf29, // 0.8134371214677901
    0x3fd3_f314_a9d1_d90a, // 0.311711469497269
    0x3fee_b6a3_e050_8229, // 0.9597949391500765
    0x3fe6_7f95_9c5c_4c24, // 0.7030742696682677
    0x3fe9_f6ed_0e58_cc20, // 0.8113923340046121
    0x3fe2_bc32_b969_7be6, // 0.5854734059647597
    0x3feb_e3b9_2360_c742, // 0.871548241708503
    0x3fdf_864b_c718_1aee, // 0.49257177775181915
    0x3feb_b238_1f80_fb76, // 0.8655052771863285
];

/// pin 配列を期待 f64 列へ変換（比較は bits 完全一致で行う）。
fn expected(bits: &[u64; 10]) -> [f64; 10] {
    let mut out = [0.0f64; 10];
    for (slot, &b) in out.iter_mut().zip(bits.iter()) {
        *slot = f64::from_bits(b);
    }
    out
}

// =====================================================================
// 契約 1: Java 実機照合 pin（JDK nextDouble とのビット完全一致）
// =====================================================================

/// `JavaRandom::new(42)` からの連続 `unit()` が JDK `new Random(42)`
/// nextDouble × 10 値とビット完全一致（呼び出し順含む）。
#[test]
fn nextdouble_bit_exact_seed_42() {
    let mut rng = JavaRandom::new(42);
    for (i, &want) in expected(&PINS_SEED_42).iter().enumerate() {
        let got = rng.unit();
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "seed 42 の {} 番目 unit(): want 0x{:016x}, got 0x{:016x}",
            i,
            want.to_bits(),
            got.to_bits()
        );
    }
}

/// `JavaRandom::new(0)` — seed 0（scramble の XOR のみで種が立つ経路）も
/// JDK 実機照合値とビット完全一致。
#[test]
fn nextdouble_bit_exact_seed_0() {
    let mut rng = JavaRandom::new(0);
    for (i, &want) in expected(&PINS_SEED_0).iter().enumerate() {
        let got = rng.unit();
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "seed 0 の {} 番目 unit(): want 0x{:016x}, got 0x{:016x}",
            i,
            want.to_bits(),
            got.to_bits()
        );
    }
}

/// `JavaRandom::new(-42)` — 負 seed（初回 scramble の内部マスク正規化経路）も
/// JDK 実機照合値とビット完全一致。
#[test]
fn nextdouble_bit_exact_seed_negative_42() {
    let mut rng = JavaRandom::new(-42);
    for (i, &want) in expected(&PINS_SEED_NEGATIVE_42).iter().enumerate() {
        let got = rng.unit();
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "seed -42 の {} 番目 unit(): want 0x{:016x}, got 0x{:016x}",
            i,
            want.to_bits(),
            got.to_bits()
        );
    }
}

/// 大きい正 seed（987654321098765432・mask 超過ビットを持つ）も
/// JDK 実機照合値とビット完全一致。
#[test]
fn nextdouble_bit_exact_seed_large_positive() {
    let mut rng = JavaRandom::new(987_654_321_098_765_432);
    for (i, &want) in expected(&PINS_SEED_LARGE).iter().enumerate() {
        let got = rng.unit();
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "seed 987654321098765432 の {} 番目 unit(): want 0x{:016x}, got 0x{:016x}",
            i,
            want.to_bits(),
            got.to_bits()
        );
    }
}

// =====================================================================
// 契約 2: 範囲 [0, 1)（焼き込み値に対する範囲 assert）
// =====================================================================

/// 全 pin 値（4 seed × 10 値 = 40 個）が [0, 1) に収まる
/// （Java nextDouble の半開区間契約・下端含み上端排他）。
#[test]
fn all_baked_pins_are_in_unit_interval() {
    for (label, bits) in [
        ("seed 42", &PINS_SEED_42),
        ("seed 0", &PINS_SEED_0),
        ("seed -42", &PINS_SEED_NEGATIVE_42),
        ("seed large", &PINS_SEED_LARGE),
    ] {
        for (i, &b) in bits.iter().enumerate() {
            let v = f64::from_bits(b);
            assert!(
                (0.0..1.0).contains(&v),
                "{} の {} 番目 pin が [0,1) 外: 0x{:016x}",
                label,
                i,
                b
            );
        }
    }
}

// =====================================================================
// 契約 3: Box<dyn Rng> として構築できる（Manager 注入契約）
// =====================================================================

/// `JavaRandom` は `Box<dyn Rng>` に格上げでき、trait object 経由の
/// 連続呼び出しでも同一 pin 列（seed 42 × 10 値）になる。
#[test]
fn usable_as_box_dyn_rng() {
    let mut rng: Box<dyn Rng> = Box::new(JavaRandom::new(42));
    for (i, &want) in expected(&PINS_SEED_42).iter().enumerate() {
        let got = rng.unit();
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "Box<dyn Rng> 経由 seed 42 の {} 番目 unit(): want 0x{:016x}, got 0x{:016x}",
            i,
            want.to_bits(),
            got.to_bits()
        );
    }
}

// =====================================================================
// 契約 4: from_os 構築（シード品質の smoke pin）
// =====================================================================

/// `JavaRandom::from_os()` 構築後の連続 `unit()` 8 値が全部同一にならず、
/// 各値が [0, 1) に収まる（統計的分布テストは意図的に行わない）。
#[test]
fn from_os_sequence_is_not_constant() {
    let mut rng = JavaRandom::from_os();
    let mut values = [0.0f64; 8];
    for slot in values.iter_mut() {
        *slot = rng.unit();
    }
    for (i, &v) in values.iter().enumerate() {
        assert!(
            (0.0..1.0).contains(&v),
            "from_os の {} 番目 unit() が [0,1) 外: 0x{:016x}",
            i,
            v.to_bits()
        );
    }
    assert!(
        values.iter().any(|&v| v.to_bits() != values[0].to_bits()),
        "from_os の連続 unit() が全同値（シードが固定化している疑い）"
    );
}

// =====================================================================
// 契約 5: 連続 from_os 構築の seed 衝突なし（#10b-2a 差戻し追加）
// =====================================================================

/// 同プロセス内で `from_os()` を連続 5 回構築し、それぞれの先頭 `unit()`
/// を取る → 5 値がすべて異なる。pid ^ 時計 nanos のみの修正前実装では
/// 時計量子以下の連続構築で同 seed となり衝突する（RED 相当）。
#[test]
fn consecutive_from_os_first_values_are_distinct() {
    let mut firsts = [0.0f64; 5];
    for slot in firsts.iter_mut() {
        let mut rng = JavaRandom::from_os();
        *slot = rng.unit();
    }
    for (i, &a) in firsts.iter().enumerate() {
        for &b in &firsts[i + 1..] {
            assert_ne!(
                a.to_bits(),
                b.to_bits(),
                "連続 from_os 構築の先頭 unit() が衝突: {} 番目 0x{:016x} と \
                 以後の値 0x{:016x}（seed 衝突の疑い）",
                i,
                a.to_bits(),
                b.to_bits()
            );
        }
    }
}
