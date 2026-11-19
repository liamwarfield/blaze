/*
 * BLAZE PATCH 0056: `java.util.TimSort` (the sort of `Arrays.sort(Object[], Comparator)`, which
 * Bazel's `sorted` uses), ported so that `sorted` compares the same pairs in the same order as
 * Bazel: the first pair that cannot be compared names the error ("unsupported comparison:
 * string <=> int"), and a comparison that fails mid-sort stops the sort instead of reaching
 * Rust's sort with an inconsistent order (which panics).
 *
 * Java: java.base/java/util/TimSort.java (OpenJDK; unchanged since JDK 8 in the parts that
 * decide which elements are compared). The scratch array is sized as needed: its size never
 * changes which elements are compared.
 */

use std::cmp::Ordering;

const MIN_MERGE: usize = 32;
const MIN_GALLOP: usize = 7;

/// Java `TimSort.sort(a, 0, a.length, c, null, 0, 0)`. `c` is Java's `Comparator.compare`; its
/// first error stops the sort and is returned.
pub(crate) fn sort<T: Copy, E>(
    a: &mut [T],
    c: &mut dyn FnMut(T, T) -> Result<Ordering, E>,
) -> Result<(), E> {
    let hi = a.len();
    let mut lo = 0;
    let mut n_remaining = hi - lo;
    if n_remaining < 2 {
        return Ok(());
    }
    if n_remaining < MIN_MERGE {
        let init_run_len = count_run_and_make_ascending(a, lo, hi, c)?;
        binary_sort(a, lo, hi, lo + init_run_len, c)?;
        return Ok(());
    }
    let mut ts = TimSort {
        tmp: Vec::new(),
        min_gallop: MIN_GALLOP,
        run_base: Vec::new(),
        run_len: Vec::new(),
    };
    let min_run = min_run_length(n_remaining);
    loop {
        let mut run_len = count_run_and_make_ascending(a, lo, hi, c)?;
        if run_len < min_run {
            let force = if n_remaining <= min_run {
                n_remaining
            } else {
                min_run
            };
            binary_sort(a, lo, lo + force, lo + run_len, c)?;
            run_len = force;
        }
        ts.run_base.push(lo);
        ts.run_len.push(run_len);
        ts.merge_collapse(a, c)?;
        lo += run_len;
        n_remaining -= run_len;
        if n_remaining == 0 {
            break;
        }
    }
    ts.merge_force_collapse(a, c)?;
    Ok(())
}

fn lt<T: Copy, E>(c: &mut dyn FnMut(T, T) -> Result<Ordering, E>, x: T, y: T) -> Result<bool, E> {
    Ok(c(x, y)? == Ordering::Less)
}

fn gt<T: Copy, E>(c: &mut dyn FnMut(T, T) -> Result<Ordering, E>, x: T, y: T) -> Result<bool, E> {
    Ok(c(x, y)? == Ordering::Greater)
}

fn binary_sort<T: Copy, E>(
    a: &mut [T],
    lo: usize,
    hi: usize,
    mut start: usize,
    c: &mut dyn FnMut(T, T) -> Result<Ordering, E>,
) -> Result<(), E> {
    if start == lo {
        start += 1;
    }
    while start < hi {
        let pivot = a[start];
        let mut left = lo;
        let mut right = start;
        while left < right {
            let mid = (left + right) >> 1;
            if lt(c, pivot, a[mid])? {
                right = mid;
            } else {
                left = mid + 1;
            }
        }
        a.copy_within(left..start, left + 1);
        a[left] = pivot;
        start += 1;
    }
    Ok(())
}

fn count_run_and_make_ascending<T: Copy, E>(
    a: &mut [T],
    lo: usize,
    hi: usize,
    c: &mut dyn FnMut(T, T) -> Result<Ordering, E>,
) -> Result<usize, E> {
    let mut run_hi = lo + 1;
    if run_hi == hi {
        return Ok(1);
    }
    let descending = lt(c, a[run_hi], a[lo])?;
    run_hi += 1;
    if descending {
        while run_hi < hi && lt(c, a[run_hi], a[run_hi - 1])? {
            run_hi += 1;
        }
        a[lo..run_hi].reverse();
    } else {
        while run_hi < hi && !lt(c, a[run_hi], a[run_hi - 1])? {
            run_hi += 1;
        }
    }
    Ok(run_hi - lo)
}

fn min_run_length(mut n: usize) -> usize {
    let mut r = 0;
    while n >= MIN_MERGE {
        r |= n & 1;
        n >>= 1;
    }
    n + r
}

/// `gallopLeft(key, a, base, len, hint, c)` over `slice[base..base + len]`.
fn gallop_left<T: Copy, E>(
    key: T,
    s: &[T],
    base: usize,
    len: usize,
    hint: usize,
    c: &mut dyn FnMut(T, T) -> Result<Ordering, E>,
) -> Result<usize, E> {
    let mut last_ofs: isize = 0;
    let mut ofs: isize = 1;
    let (len, hint) = (len as isize, hint as isize);
    let at = |i: isize| s[(base as isize + i) as usize];
    if gt(c, key, at(hint))? {
        let max_ofs = len - hint;
        while ofs < max_ofs && gt(c, key, at(hint + ofs))? {
            last_ofs = ofs;
            ofs = (ofs << 1) + 1;
        }
        if ofs > max_ofs {
            ofs = max_ofs;
        }
        last_ofs += hint;
        ofs += hint;
    } else {
        let max_ofs = hint + 1;
        while ofs < max_ofs && !gt(c, key, at(hint - ofs))? {
            last_ofs = ofs;
            ofs = (ofs << 1) + 1;
        }
        if ofs > max_ofs {
            ofs = max_ofs;
        }
        let tmp = last_ofs;
        last_ofs = hint - ofs;
        ofs = hint - tmp;
    }
    last_ofs += 1;
    while last_ofs < ofs {
        let m = last_ofs + ((ofs - last_ofs) >> 1);
        if gt(c, key, at(m))? {
            last_ofs = m + 1;
        } else {
            ofs = m;
        }
    }
    Ok(ofs as usize)
}

/// `gallopRight(key, a, base, len, hint, c)` over `slice[base..base + len]`.
fn gallop_right<T: Copy, E>(
    key: T,
    s: &[T],
    base: usize,
    len: usize,
    hint: usize,
    c: &mut dyn FnMut(T, T) -> Result<Ordering, E>,
) -> Result<usize, E> {
    let mut ofs: isize = 1;
    let mut last_ofs: isize = 0;
    let (len, hint) = (len as isize, hint as isize);
    let at = |i: isize| s[(base as isize + i) as usize];
    if lt(c, key, at(hint))? {
        let max_ofs = hint + 1;
        while ofs < max_ofs && lt(c, key, at(hint - ofs))? {
            last_ofs = ofs;
            ofs = (ofs << 1) + 1;
        }
        if ofs > max_ofs {
            ofs = max_ofs;
        }
        let tmp = last_ofs;
        last_ofs = hint - ofs;
        ofs = hint - tmp;
    } else {
        let max_ofs = len - hint;
        while ofs < max_ofs && !lt(c, key, at(hint + ofs))? {
            last_ofs = ofs;
            ofs = (ofs << 1) + 1;
        }
        if ofs > max_ofs {
            ofs = max_ofs;
        }
        last_ofs += hint;
        ofs += hint;
    }
    last_ofs += 1;
    while last_ofs < ofs {
        let m = last_ofs + ((ofs - last_ofs) >> 1);
        if lt(c, key, at(m))? {
            ofs = m;
        } else {
            last_ofs = m + 1;
        }
    }
    Ok(ofs as usize)
}

struct TimSort<T> {
    tmp: Vec<T>,
    min_gallop: usize,
    run_base: Vec<usize>,
    run_len: Vec<usize>,
}

impl<T: Copy> TimSort<T> {
    fn merge_collapse<E>(
        &mut self,
        a: &mut [T],
        c: &mut dyn FnMut(T, T) -> Result<Ordering, E>,
    ) -> Result<(), E> {
        while self.run_len.len() > 1 {
            let mut n = self.run_len.len() - 2;
            let rl = &self.run_len;
            if n > 0 && rl[n - 1] <= rl[n] + rl[n + 1] || n > 1 && rl[n - 2] <= rl[n] + rl[n - 1] {
                if rl[n - 1] < rl[n + 1] {
                    n -= 1;
                }
            } else if rl[n] > rl[n + 1] {
                break;
            }
            self.merge_at(a, n, c)?;
        }
        Ok(())
    }

    fn merge_force_collapse<E>(
        &mut self,
        a: &mut [T],
        c: &mut dyn FnMut(T, T) -> Result<Ordering, E>,
    ) -> Result<(), E> {
        while self.run_len.len() > 1 {
            let mut n = self.run_len.len() - 2;
            if n > 0 && self.run_len[n - 1] < self.run_len[n + 1] {
                n -= 1;
            }
            self.merge_at(a, n, c)?;
        }
        Ok(())
    }

    fn merge_at<E>(
        &mut self,
        a: &mut [T],
        i: usize,
        c: &mut dyn FnMut(T, T) -> Result<Ordering, E>,
    ) -> Result<(), E> {
        let mut base1 = self.run_base[i];
        let mut len1 = self.run_len[i];
        let base2 = self.run_base[i + 1];
        let mut len2 = self.run_len[i + 1];
        self.run_len[i] = len1 + len2;
        self.run_base.remove(i + 1);
        self.run_len.remove(i + 1);
        let k = gallop_right(a[base2], a, base1, len1, 0, c)?;
        base1 += k;
        len1 -= k;
        if len1 == 0 {
            return Ok(());
        }
        len2 = gallop_left(a[base1 + len1 - 1], a, base2, len2, len2 - 1, c)?;
        if len2 == 0 {
            return Ok(());
        }
        if len1 <= len2 {
            self.merge_lo(a, base1, len1, base2, len2, c)
        } else {
            self.merge_hi(a, base1, len1, base2, len2, c)
        }
    }

    fn ensure_capacity(&mut self, a: &[T], min_capacity: usize) {
        if self.tmp.len() < min_capacity {
            self.tmp.resize(min_capacity, a[0]);
        }
    }

    fn merge_lo<E>(
        &mut self,
        a: &mut [T],
        base1: usize,
        mut len1: usize,
        base2: usize,
        mut len2: usize,
        c: &mut dyn FnMut(T, T) -> Result<Ordering, E>,
    ) -> Result<(), E> {
        self.ensure_capacity(a, len1);
        let tmp = &mut self.tmp;
        let mut cursor1 = 0;
        let mut cursor2 = base2;
        let mut dest = base1;
        tmp[cursor1..cursor1 + len1].copy_from_slice(&a[base1..base1 + len1]);
        a[dest] = a[cursor2];
        dest += 1;
        cursor2 += 1;
        len2 -= 1;
        if len2 == 0 {
            a[dest..dest + len1].copy_from_slice(&tmp[cursor1..cursor1 + len1]);
            return Ok(());
        }
        if len1 == 1 {
            a.copy_within(cursor2..cursor2 + len2, dest);
            a[dest + len2] = tmp[cursor1];
            return Ok(());
        }
        let mut min_gallop = self.min_gallop as isize;
        'outer: loop {
            let mut count1: usize = 0;
            let mut count2: usize = 0;
            loop {
                if lt(c, a[cursor2], tmp[cursor1])? {
                    a[dest] = a[cursor2];
                    dest += 1;
                    cursor2 += 1;
                    count2 += 1;
                    count1 = 0;
                    len2 -= 1;
                    if len2 == 0 {
                        break 'outer;
                    }
                } else {
                    a[dest] = tmp[cursor1];
                    dest += 1;
                    cursor1 += 1;
                    count1 += 1;
                    count2 = 0;
                    len1 -= 1;
                    if len1 == 1 {
                        break 'outer;
                    }
                }
                if ((count1 | count2) as isize) >= min_gallop {
                    break;
                }
            }
            loop {
                count1 = gallop_right(a[cursor2], tmp, cursor1, len1, 0, c)?;
                if count1 != 0 {
                    a[dest..dest + count1].copy_from_slice(&tmp[cursor1..cursor1 + count1]);
                    dest += count1;
                    cursor1 += count1;
                    len1 -= count1;
                    if len1 <= 1 {
                        break 'outer;
                    }
                }
                a[dest] = a[cursor2];
                dest += 1;
                cursor2 += 1;
                len2 -= 1;
                if len2 == 0 {
                    break 'outer;
                }
                count2 = gallop_left(tmp[cursor1], a, cursor2, len2, 0, c)?;
                if count2 != 0 {
                    a.copy_within(cursor2..cursor2 + count2, dest);
                    dest += count2;
                    cursor2 += count2;
                    len2 -= count2;
                    if len2 == 0 {
                        break 'outer;
                    }
                }
                a[dest] = tmp[cursor1];
                dest += 1;
                cursor1 += 1;
                len1 -= 1;
                if len1 == 1 {
                    break 'outer;
                }
                min_gallop -= 1;
                if !(count1 >= MIN_GALLOP || count2 >= MIN_GALLOP) {
                    break;
                }
            }
            if min_gallop < 0 {
                min_gallop = 0;
            }
            min_gallop += 2;
        }
        self.min_gallop = if min_gallop < 1 {
            1
        } else {
            min_gallop as usize
        };
        if len1 == 1 {
            a.copy_within(cursor2..cursor2 + len2, dest);
            a[dest + len2] = tmp[cursor1];
        } else if len1 == 0 {
            // Java throws IllegalArgumentException; Starlark's ordering is consistent.
            panic!("Comparison method violates its general contract!");
        } else {
            a[dest..dest + len1].copy_from_slice(&tmp[cursor1..cursor1 + len1]);
        }
        Ok(())
    }

    fn merge_hi<E>(
        &mut self,
        a: &mut [T],
        base1: usize,
        mut len1: usize,
        base2: usize,
        mut len2: usize,
        c: &mut dyn FnMut(T, T) -> Result<Ordering, E>,
    ) -> Result<(), E> {
        self.ensure_capacity(a, len2);
        let tmp = &mut self.tmp;
        let tmp_base: isize = 0;
        tmp[0..len2].copy_from_slice(&a[base2..base2 + len2]);
        // Cursors and dest go below their bases (to -1), as in Java.
        let mut cursor1 = (base1 + len1) as isize - 1;
        let mut cursor2 = tmp_base + len2 as isize - 1;
        let mut dest = (base2 + len2) as isize - 1;
        let u = |i: isize| i as usize;
        a[u(dest)] = a[u(cursor1)];
        dest -= 1;
        cursor1 -= 1;
        len1 -= 1;
        if len1 == 0 {
            let d = u(dest - (len2 as isize - 1));
            a[d..d + len2].copy_from_slice(&tmp[0..len2]);
            return Ok(());
        }
        if len2 == 1 {
            dest -= len1 as isize;
            cursor1 -= len1 as isize;
            a.copy_within(u(cursor1 + 1)..u(cursor1 + 1) + len1, u(dest + 1));
            a[u(dest)] = tmp[u(cursor2)];
            return Ok(());
        }
        let mut min_gallop = self.min_gallop as isize;
        'outer: loop {
            let mut count1: usize = 0;
            let mut count2: usize = 0;
            loop {
                if lt(c, tmp[u(cursor2)], a[u(cursor1)])? {
                    a[u(dest)] = a[u(cursor1)];
                    dest -= 1;
                    cursor1 -= 1;
                    count1 += 1;
                    count2 = 0;
                    len1 -= 1;
                    if len1 == 0 {
                        break 'outer;
                    }
                } else {
                    a[u(dest)] = tmp[u(cursor2)];
                    dest -= 1;
                    cursor2 -= 1;
                    count2 += 1;
                    count1 = 0;
                    len2 -= 1;
                    if len2 == 1 {
                        break 'outer;
                    }
                }
                if ((count1 | count2) as isize) >= min_gallop {
                    break;
                }
            }
            loop {
                count1 = len1 - gallop_right(tmp[u(cursor2)], a, base1, len1, len1 - 1, c)?;
                if count1 != 0 {
                    dest -= count1 as isize;
                    cursor1 -= count1 as isize;
                    len1 -= count1;
                    a.copy_within(u(cursor1 + 1)..u(cursor1 + 1) + count1, u(dest + 1));
                    if len1 == 0 {
                        break 'outer;
                    }
                }
                a[u(dest)] = tmp[u(cursor2)];
                dest -= 1;
                cursor2 -= 1;
                len2 -= 1;
                if len2 == 1 {
                    break 'outer;
                }
                count2 = len2 - gallop_left(a[u(cursor1)], tmp, u(tmp_base), len2, len2 - 1, c)?;
                if count2 != 0 {
                    dest -= count2 as isize;
                    cursor2 -= count2 as isize;
                    len2 -= count2;
                    a[u(dest + 1)..u(dest + 1) + count2]
                        .copy_from_slice(&tmp[u(cursor2 + 1)..u(cursor2 + 1) + count2]);
                    if len2 <= 1 {
                        break 'outer;
                    }
                }
                a[u(dest)] = a[u(cursor1)];
                dest -= 1;
                cursor1 -= 1;
                len1 -= 1;
                if len1 == 0 {
                    break 'outer;
                }
                min_gallop -= 1;
                if !(count1 >= MIN_GALLOP || count2 >= MIN_GALLOP) {
                    break;
                }
            }
            if min_gallop < 0 {
                min_gallop = 0;
            }
            min_gallop += 2;
        }
        self.min_gallop = if min_gallop < 1 {
            1
        } else {
            min_gallop as usize
        };
        if len2 == 1 {
            dest -= len1 as isize;
            cursor1 -= len1 as isize;
            a.copy_within(u(cursor1 + 1)..u(cursor1 + 1) + len1, u(dest + 1));
            a[u(dest)] = tmp[u(cursor2)];
        } else if len2 == 0 {
            // Java throws IllegalArgumentException; Starlark's ordering is consistent.
            panic!("Comparison method violates its general contract!");
        } else {
            let d = u(dest - (len2 as isize - 1));
            a[d..d + len2].copy_from_slice(&tmp[0..len2]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sorts like a stable sort for every length around the run and merge thresholds.
    #[test]
    fn sorts_stably() {
        let mut seed: u64 = 12345;
        for n in [0, 1, 2, 31, 32, 33, 64, 100, 257, 1000, 5000] {
            for range in [3u64, 50, 100000] {
                let mut v: Vec<(u64, usize)> = (0..n)
                    .map(|i| {
                        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                        ((seed >> 33) % range, i)
                    })
                    .collect();
                let mut expected = v.clone();
                expected.sort_by_key(|x| x.0);
                sort::<_, ()>(&mut v, &mut |x, y| Ok(x.0.cmp(&y.0))).unwrap();
                assert_eq!(v, expected, "n={n} range={range}");
            }
        }
    }
}
