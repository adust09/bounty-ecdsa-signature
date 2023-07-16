use std::time::Instant;

use num_bigint::BigInt;
use rand::Rng;
use tfhe::integer::{
    block_decomposition::{DecomposableInto, RecomposableFrom},
    IntegerCiphertext, RadixCiphertext, ServerKey, U256,
};

use crate::helper::{
    bigint_ilog2_ceil, bigint_ilog2_floor, bigint_to_u128, from_bigint, to_bigint,
};

/// Calculate n, m, p from coeff
/// `coeff` in the form of p = 2^n_0 - 2^n_1 - ... - 2^n_{k-1} - n_k
/// `c` in the form of c = 2^n_0 - p
/// `c` must be in range 0 <= c <= 2^floor(n/2)
#[inline(always)]
pub fn mersenne_coeff(coeff: &[u32]) -> (u32, BigInt, BigInt, BigInt) {
    assert!(coeff.len() > 1);
    let len = coeff.len();
    let n = coeff[0];
    let p = coeff[1..len - 1]
        .iter()
        .fold(BigInt::from(2).pow(coeff[0]), |acc, b| {
            acc - BigInt::from(2).pow(*b)
        })
        - coeff[len - 1];
    let q = BigInt::from(2).pow(n);
    let c = &q - &p;

    (n, p, q, c)
}

#[inline(always)]
/// Calculate n, c from p
/// `c` must be in range 0 <= c <= 2^floor(n/2)
pub fn mersenne_coeff_p<P: DecomposableInto<u8> + Copy + Sync>(p: P) -> (u32, BigInt) {
    let pb = to_bigint(p);
    let n = bigint_ilog2_ceil(&pb);
    let c = (BigInt::from(1) << n) - &pb;

    (n, c)
}

#[inline(always)]
/// Calculate n, m, c from p
/// `c` must be in range 0 <= c <= 2^floor(n/2)
pub fn mersenne_coeff_p2<P: DecomposableInto<u8> + Copy + Sync>(
    p: P,
) -> (u32, Option<u32>, BigInt) {
    let pb = to_bigint(p);
    let n = bigint_ilog2_ceil(&pb);
    let b = (BigInt::from(1) << n) - &pb;
    if b == BigInt::from(1) {
        return (n, None, BigInt::from(1));
    }
    let m = bigint_ilog2_floor(&b);
    let c = &b - (BigInt::from(1) << m);

    (n, Some(m), c)
}

/// Calculate x mod p^2 mod p
pub fn mersenne_mod_native(x: &BigInt, p: &BigInt) -> BigInt {
    let (n, m, c) = mersenne_coeff_p2(from_bigint::<U256>(p));

    // x = a*2^n + b
    let a = x >> n;
    let b = x - (&a << n);
    assert_eq!(*x, &a * &BigInt::from(2).pow(n) + &b);

    // x % p = (2^m+c)*a + b
    let x_mod_p = &a
        * (&c
            + match m {
                Some(m) => &BigInt::from(1) << m,
                None => BigInt::from(0),
            })
        + &b;

    println!("bits: {}", x_mod_p.bits());
    if x_mod_p >= *p {
        mersenne_mod_native(&x_mod_p, p)
    } else {
        x_mod_p
    }
}

/// Calculate x mod p^2 mod p
pub fn mod_mersenne<
    const NB: usize,
    P: DecomposableInto<u64> + DecomposableInto<u8> + Copy + Sync,
>(
    x: &RadixCiphertext,
    p: P,
    server_key: &ServerKey,
) -> RadixCiphertext {
    let (n, c) = mersenne_coeff_p(p);
    let c_blocks = (c.bits() as usize + 1) / 2;
    let x = server_key.extend_radix_with_trivial_zero_blocks_msb(x, (NB * 2) - x.blocks().len());

    // first pass NB*2 blocks
    let x_mod_p = (|x: &RadixCiphertext| {
        let mut a = server_key.scalar_right_shift_parallelized(x, n as u64);
        let mut b = server_key.smart_sub_parallelized(
            &mut x.clone(),
            &mut server_key.scalar_left_shift_parallelized(&a, n as u64),
        );

        let len = x.blocks().len();
        // a will be multiplied by c, so it must be at least NB + c_blocks long
        server_key.trim_radix_blocks_msb_assign(&mut a, len - (NB + c_blocks + 1));
        // b must be at least NB long
        server_key.trim_radix_blocks_msb_assign(&mut b, len - NB);
        let mut ca = server_key.smart_scalar_mul_parallelized(&mut a, bigint_to_u128(&c));
        server_key.add_parallelized(&mut ca, &mut b)
    })(&x);

    // second pass % NB + c_blocks blocks
    let x_mod_p2 = (|x: &RadixCiphertext| {
        let mut a = server_key.scalar_right_shift_parallelized(x, n as u64);
        let mut b = server_key.smart_sub_parallelized(
            &mut x.clone(),
            &mut server_key.scalar_left_shift_parallelized(&a, n as u64),
        );

        let len = x.blocks().len();
        // a will be multiplied by c, so it must be at least NB + 1 long
        server_key.trim_radix_blocks_msb_assign(&mut a, len - (NB + 1));
        // b must be at least NB long
        server_key.trim_radix_blocks_msb_assign(&mut b, len - NB);
        let mut ca = server_key.smart_scalar_mul_parallelized(&mut a, bigint_to_u128(&c));
        server_key.add_parallelized(&mut ca, &mut b)
    })(&x_mod_p);

    // final pass % NB + 1 blocks
    let mut x_mod_p3 = (|x: &RadixCiphertext| {
        let mut a = server_key.scalar_right_shift_parallelized(x, n as u64);
        let mut b = server_key.smart_sub_parallelized(
            &mut x.clone(),
            &mut server_key.scalar_left_shift_parallelized(&a, n as u64),
        );
        let len = x.blocks().len();
        server_key.trim_radix_blocks_msb_assign(&mut a, len - (2 + c_blocks));
        server_key.trim_radix_blocks_msb_assign(&mut b, len - NB);
        let mut ca = server_key.smart_scalar_mul_parallelized(&mut a, bigint_to_u128(&c));
        server_key.add_parallelized(&mut b, &mut ca)
    })(&x_mod_p2);

    let len = x_mod_p3.blocks().len();
    server_key.trim_radix_blocks_msb_assign(&mut x_mod_p3, len - NB);
    x_mod_p3
}

/// Calculate x mod p^2 mod p
pub fn mod_mersenne2<
    const NB: usize,
    P: DecomposableInto<u64> + DecomposableInto<u8> + Copy + Sync,
>(
    x: &RadixCiphertext,
    p: P,
    server_key: &ServerKey,
) -> RadixCiphertext {
    let (n, m, c) = mersenne_coeff_p2(p);
    let c_blocks = (c.bits() as usize + 1) / 2;
    let x = server_key.extend_radix_with_trivial_zero_blocks_msb(x, (NB * 2) - x.blocks().len());

    // first pass NB*2 blocks
    let x_mod_p = (|x: &RadixCiphertext| {
        let mut a = server_key.scalar_right_shift_parallelized(x, n as u64);
        let mut b = server_key.smart_sub_parallelized(
            &mut x.clone(),
            &mut server_key.scalar_left_shift_parallelized(&a, n as u64),
        );

        let len = x.blocks().len();
        // a will be multiplied by c, so it must be at least NB + c_blocks long
        server_key.trim_radix_blocks_msb_assign(&mut a, len - (NB + c_blocks + 1));
        // b must be at least NB long
        server_key.trim_radix_blocks_msb_assign(&mut b, len - NB);
        if let Some(m) = m {
            let (mut ma, mut ca) = rayon::join(
                || server_key.scalar_left_shift_parallelized(&a, m as u64),
                || server_key.smart_scalar_mul_parallelized(&mut a.clone(), bigint_to_u128(&c)),
            );
            server_key.smart_add_assign_parallelized(&mut ca, &mut ma);
            server_key.add_parallelized(&mut ca, &mut b)
        } else {
            let mut ca = server_key.smart_scalar_mul_parallelized(&mut a, bigint_to_u128(&c));
            server_key.add_parallelized(&mut ca, &mut b)
        }
    })(&x);

    // second pass % NB + c_blocks blocks
    let x_mod_p2 = (|x: &RadixCiphertext| {
        let mut a = server_key.scalar_right_shift_parallelized(x, n as u64);
        let mut b = server_key.smart_sub_parallelized(
            &mut x.clone(),
            &mut server_key.scalar_left_shift_parallelized(&a, n as u64),
        );

        let len = x.blocks().len();
        // a will be multiplied by c, so it must be at least NB + 1 long
        server_key.trim_radix_blocks_msb_assign(&mut a, len - (NB + 1));
        // b must be at least NB long
        server_key.trim_radix_blocks_msb_assign(&mut b, len - NB);
        if let Some(m) = m {
            let (mut ma, mut ca) = rayon::join(
                || server_key.scalar_left_shift_parallelized(&a, m as u64),
                || server_key.smart_scalar_mul_parallelized(&mut a.clone(), bigint_to_u128(&c)),
            );
            server_key.smart_add_assign_parallelized(&mut ca, &mut ma);
            server_key.add_parallelized(&mut ca, &mut b)
        } else {
            let mut ca = server_key.smart_scalar_mul_parallelized(&mut a, bigint_to_u128(&c));
            server_key.add_parallelized(&mut ca, &mut b)
        }
    })(&x_mod_p);

    // final pass % NB + 1 blocks
    let mut x_mod_p3 = (|x: &RadixCiphertext| {
        let mut a = server_key.scalar_right_shift_parallelized(x, n as u64);
        let mut b = server_key.smart_sub_parallelized(
            &mut x.clone(),
            &mut server_key.scalar_left_shift_parallelized(&a, n as u64),
        );
        let len = x.blocks().len();
        server_key.trim_radix_blocks_msb_assign(&mut a, len - (2 + c_blocks));
        server_key.trim_radix_blocks_msb_assign(&mut b, len - NB);
        if let Some(m) = m {
            let (mut ma, mut ca) = rayon::join(
                || server_key.scalar_left_shift_parallelized(&a, m as u64),
                || server_key.smart_scalar_mul_parallelized(&mut a.clone(), bigint_to_u128(&c)),
            );
            server_key.smart_add_assign_parallelized(&mut ca, &mut ma);
            server_key.add_parallelized(&mut ca, &mut b)
        } else {
            let mut ca = server_key.smart_scalar_mul_parallelized(&mut a, bigint_to_u128(&c));
            server_key.add_parallelized(&mut ca, &mut b)
        }
    })(&x_mod_p2);

    let len = x_mod_p3.blocks().len();
    server_key.trim_radix_blocks_msb_assign(&mut x_mod_p3, len - NB);
    x_mod_p3
}

/// Calculate x mod 2p mod p
pub fn mod_mersenne_fast<
    const NB: usize,
    P: DecomposableInto<u64> + DecomposableInto<u8> + Copy + Sync,
>(
    x: &RadixCiphertext,
    p: P,
    server_key: &ServerKey,
) -> RadixCiphertext {
    let (n, c) = mersenne_coeff_p(p);
    let c_blocks = (c.bits() as usize + 1) / 2;
    let mut a = server_key.scalar_right_shift_parallelized(x, n as u64);
    let len = x.blocks().len();
    let mut b = server_key.smart_sub_parallelized(
        &mut x.clone(),
        &mut server_key.scalar_left_shift_parallelized(&a, n as u64),
    );
    // a must be at least 2 + c_blocks (1 + c_bits bits) long
    server_key.trim_radix_blocks_msb_assign(&mut a, len - (2 + c_blocks));
    // b must be at least NB long
    server_key.trim_radix_blocks_msb_assign(&mut b, len - NB);
    let mut ca = server_key.smart_scalar_mul_parallelized(&mut a, bigint_to_u128(&c));
    let mut x_mod_p = server_key.add_parallelized(&mut b, &mut ca);
    let len = x_mod_p.blocks().len();
    server_key.trim_radix_blocks_msb_assign(&mut x_mod_p, len - NB);
    x_mod_p
}

/// Calculate a * b mod p
pub fn mul_mod_mersenne<
    const NB: usize,
    P: DecomposableInto<u64> + RecomposableFrom<u64> + DecomposableInto<u8> + Copy + Sync,
>(
    a: &RadixCiphertext,
    b: &RadixCiphertext,
    p: P,
    server_key: &ServerKey,
) -> RadixCiphertext {
    #[cfg(feature = "low_level_timing")]
    let ops_start = Instant::now();
    #[cfg(feature = "low_level_timing")]
    let task_ref = rand::thread_rng().gen_range(0..1000);
    #[cfg(feature = "low_level_timing")]
    println!("mul mod mersenne start -- ref {}", task_ref);

    let mut a_expanded = server_key.extend_radix_with_trivial_zero_blocks_msb(a, NB);
    server_key.smart_mul_assign_parallelized(&mut a_expanded, &mut b.clone());
    //server_key.full_propagate_parallelized(&mut a_expanded);
    let res = mod_mersenne::<NB, _>(&a_expanded, p, server_key);
    #[cfg(feature = "low_level_timing")]
    println!(
        "mul mod mersenne done in {:.2}s -- ref {}",
        ops_start.elapsed().as_secs_f64(),
        task_ref
    );
    res
}

/// Calculate a * b mod p
pub fn mul_mod_mersenne2<
    const NB: usize,
    P: DecomposableInto<u64> + RecomposableFrom<u64> + DecomposableInto<u8> + Copy + Sync,
>(
    a: &RadixCiphertext,
    b: &RadixCiphertext,
    p: P,
    server_key: &ServerKey,
) -> RadixCiphertext {
    #[cfg(feature = "low_level_timing")]
    let ops_start = Instant::now();
    #[cfg(feature = "low_level_timing")]
    let task_ref = rand::thread_rng().gen_range(0..1000);
    #[cfg(feature = "low_level_timing")]
    println!("mul mod mersenne start -- ref {}", task_ref);

    let mut a_expanded = server_key.extend_radix_with_trivial_zero_blocks_msb(a, NB);
    server_key.smart_mul_assign_parallelized(&mut a_expanded, &mut b.clone());
    //server_key.full_propagate_parallelized(&mut a_expanded);
    let res = mod_mersenne2::<NB, _>(&a_expanded, p, server_key);
    #[cfg(feature = "low_level_timing")]
    println!(
        "mul mod mersenne done in {:.2}s -- ref {}",
        ops_start.elapsed().as_secs_f64(),
        task_ref
    );
    res
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use num_bigint::BigInt;
    use tfhe::{integer::keycache::IntegerKeyCache, shortint::prelude::PARAM_MESSAGE_2_CARRY_2};

    use crate::ops::{
        mersenne::{mersenne_mod_native, mod_mersenne_fast, mul_mod_mersenne, mul_mod_mersenne2},
        native::mul_mod_native,
    };

    use super::{mersenne_coeff, mersenne_coeff_p};

    #[test]
    fn correct_mersenne_native_mod() {
        let p = 251u8;
        let x = 249u8;
        let y = 248u8;

        assert_eq!(
            mersenne_mod_native(&(&BigInt::from(x) * &BigInt::from(y)), &BigInt::from(p)),
            BigInt::from(mul_mod_native(x, y, p))
        );
    }

    #[test]
    fn correct_mersenne_mul_mod() {
        let (client_key, server_key) = IntegerKeyCache.get_from_params(PARAM_MESSAGE_2_CARRY_2);
        const NUM_BLOCK: usize = 4;
        let p: u8 = 251;

        let mul_mod_naive = |x: u128, y: u128| -> u128 { (x * y) % p as u128 };

        let x: u128 = 249;
        let y: u128 = 248;
        let enc_x = client_key.encrypt_radix(x, NUM_BLOCK);
        let enc_y = client_key.encrypt_radix(y, NUM_BLOCK);
        let now = Instant::now();
        let xy_mod_p = mul_mod_mersenne::<NUM_BLOCK, _>(&enc_x, &enc_y, p, &server_key);
        println!(
            "mul mod mersenne done in {:.2}s",
            now.elapsed().as_secs_f64()
        );
        assert_eq!(
            client_key.decrypt_radix::<u128>(&xy_mod_p),
            mul_mod_naive(x, y)
        );
        let now = Instant::now();
        let xy_mod_p = mul_mod_mersenne2::<NUM_BLOCK, _>(&enc_x, &enc_y, p, &server_key);
        println!(
            "mul mod mersenne2 done in {:.2}s",
            now.elapsed().as_secs_f64()
        );
        assert_eq!(
            client_key.decrypt_radix::<u128>(&xy_mod_p),
            mul_mod_naive(x, y)
        );
    }

    #[test]
    fn correct_mersenne_transfrom() {
        let p = 127;
        let coeff = mersenne_coeff_p(p);
        assert_eq!(coeff, (7, BigInt::from(1)));
    }

    #[test]
    fn correct_mersenne_mod_fast() {
        let (client_key, server_key) = IntegerKeyCache.get_from_params(PARAM_MESSAGE_2_CARRY_2);

        const NUM_BLOCK: usize = 4;
        let p: u128 = 251;
        let x1: u128 = 249;
        let ct_x1 = client_key.encrypt_radix(x1, NUM_BLOCK + 1);
        let now = Instant::now();
        let added = server_key.add_parallelized(&ct_x1, &ct_x1);
        let res = mod_mersenne_fast::<NUM_BLOCK, _>(&added, p, &server_key);
        println!(
            "mod mersenne fast done in {:.2}s",
            now.elapsed().as_secs_f64()
        );
        let dec_res = client_key.decrypt_radix::<u128>(&res);
        assert_eq!(dec_res, (x1 * 2) % p);
    }
}
