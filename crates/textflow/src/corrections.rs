//! 主人在历史里把一句改对了：比对改前改后，挑出改动的词，变成热词表的「错=对」规则。
//!
//! - 按字比对（最长公共子序列），连着改动的一段算一处；
//! - 英文改到半个词时扩成整个词（Cloud → Claude，而不是 o → au）；
//! - 只改了一个字时带上旁边一个字（雪而=雪耳），免得「而=耳」把所有的「而」都改掉；
//! - 只改标点、空格、大小写，或者纯加字、删字的不算（不是听错了）；
//! - 两边最多 40 个字、不能有 = 和换行（热词表一行的规矩，见 `add_hotword_line`）。

/// 改前改后，返回要记住的（错, 对），按出现顺序、不重复
pub fn corrections(before: &str, after: &str) -> Vec<(String, String)> {
    let a: Vec<char> = before.chars().collect();
    let b: Vec<char> = after.chars().collect();
    // 太长的不比（一句话不会这么长，按字比对是平方的）
    if a == b || a.len() * b.len() > 250_000 {
        return Vec::new();
    }
    let mut regions = changed(&a, &b);
    for region in &mut regions {
        whole_words(region, &a, &b);
    }
    let regions = merged(regions);
    let mut out: Vec<(String, String)> = Vec::new();
    for mut region in regions {
        widen_single(&mut region, &a, &b);
        let wrong: String = a[region.0..region.1].iter().collect();
        let right: String = b[region.2..region.3].iter().collect();
        let (wrong, right) = (wrong.trim().to_owned(), right.trim().to_owned());
        if usable(&wrong, &right) && !out.iter().any(|(w, _)| *w == wrong) {
            out.push((wrong, right));
        }
    }
    out
}

/// 改动的地方：（改前起, 改前止, 改后起, 改后止），两段之间至少隔一个没改的字
type Region = (usize, usize, usize, usize);

fn changed(a: &[char], b: &[char]) -> Vec<Region> {
    let (n, m) = (a.len(), b.len());
    // common[i][j]：a[i..] 和 b[j..] 的最长公共子序列有多长
    let mut common = vec![vec![0_u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            common[i][j] = if a[i] == b[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut regions = Vec::new();
    let mut open: Option<(usize, usize)> = None;
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            if let Some((a0, b0)) = open.take() {
                regions.push((a0, i, b0, j));
            }
            i += 1;
            j += 1;
        } else {
            open.get_or_insert((i, j));
            if j < m && (i == n || common[i][j + 1] >= common[i + 1][j]) {
                j += 1;
            } else {
                i += 1;
            }
        }
    }
    if let Some((a0, b0)) = open {
        regions.push((a0, n, b0, m));
    }
    regions
}

/// 改动碰到英文字母或数字时，往两边扩到整个词
fn whole_words(region: &mut Region, a: &[char], b: &[char]) {
    let word = |c: char| c.is_ascii_alphanumeric();
    let touches = |r: &Region| {
        a.get(r.0).is_some_and(|c| word(*c))
            || b.get(r.2).is_some_and(|c| word(*c))
            || (r.1 > r.0 && word(a[r.1 - 1]))
            || (r.3 > r.2 && word(b[r.3 - 1]))
    };
    if !touches(region) {
        return;
    }
    while region.0 > 0
        && region.2 > 0
        && a[region.0 - 1] == b[region.2 - 1]
        && word(a[region.0 - 1])
    {
        region.0 -= 1;
        region.2 -= 1;
    }
    while region.1 < a.len()
        && region.3 < b.len()
        && a[region.1] == b[region.3]
        && word(a[region.1])
    {
        region.1 += 1;
        region.3 += 1;
    }
}

/// 扩成整词后重叠或挨着的几段并成一段
fn merged(mut regions: Vec<Region>) -> Vec<Region> {
    regions.sort();
    let mut out: Vec<Region> = Vec::new();
    for r in regions {
        match out.last_mut() {
            Some(last) if r.0 <= last.1 || r.2 <= last.3 => {
                *last = (
                    last.0.min(r.0),
                    last.1.max(r.1),
                    last.2.min(r.2),
                    last.3.max(r.3),
                );
            }
            _ => out.push(r),
        }
    }
    out
}

/// 两边都改了、改前只有一个字（不算标点）：带上左边（没有就右边）一个没改的字
fn widen_single(region: &mut Region, a: &[char], b: &[char]) {
    let letters = |s: &[char]| s.iter().filter(|c| c.is_alphanumeric()).count();
    if letters(&a[region.0..region.1]) != 1 || letters(&b[region.2..region.3]) == 0 {
        return;
    }
    if region.0 > 0
        && region.2 > 0
        && a[region.0 - 1] == b[region.2 - 1]
        && a[region.0 - 1].is_alphanumeric()
    {
        region.0 -= 1;
        region.2 -= 1;
    } else if region.1 < a.len()
        && region.3 < b.len()
        && a[region.1] == b[region.3]
        && a[region.1].is_alphanumeric()
    {
        region.1 += 1;
        region.3 += 1;
    }
}

fn usable(wrong: &str, right: &str) -> bool {
    let core = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let fits = |s: &str| s.chars().count() <= 40 && !s.contains(['=', '\n', '\r', '\t']);
    let (w, r) = (core(wrong), core(right));
    !w.is_empty()
        && !r.is_empty()
        && w != r
        && fits(wrong)
        && fits(right)
        && !wrong.starts_with('#')
}
