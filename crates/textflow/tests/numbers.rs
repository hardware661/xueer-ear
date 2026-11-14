use xueer_ear_textflow::normalize_numbers;

#[test]
fn percent() {
    assert_eq!(normalize_numbers("完成了百分之五十"), "完成了50%");
    assert_eq!(normalize_numbers("百分之三点五"), "3.5%");
}

#[test]
fn decimals() {
    assert_eq!(normalize_numbers("版本三点五"), "版本3.5");
}

#[test]
fn digit_by_digit_years() {
    assert_eq!(normalize_numbers("二零二六年十一月"), "2026年11月");
}

#[test]
fn multi_digit_with_units() {
    assert_eq!(normalize_numbers("一共三百二十五个"), "一共325个");
    assert_eq!(normalize_numbers("一万两千"), "12000");
    assert_eq!(normalize_numbers("十二点半"), "12点半");
    assert_eq!(normalize_numbers("两千"), "2000");
}

#[test]
fn leaves_idioms_and_single_digits_alone() {
    for s in [
        "看一下",
        "有一些",
        "一起走",
        "三心二意",
        "一个文件",
        "第一次",
        "两个人",
        "十分好",
        "千万别忘了",
        "万一出错",
        "百万",
        "五六个",
        "一一检查",
        "三三两两",
        "一五一十",
        "四五十个",
        "九九八十一",
    ] {
        assert_eq!(normalize_numbers(s), s, "{s}");
    }
}

#[test]
fn spoken_shortcuts_and_zero() {
    assert_eq!(normalize_numbers("一万二"), "12000");
    assert_eq!(normalize_numbers("三百五"), "350");
    assert_eq!(normalize_numbers("三百零五"), "305");
    assert_eq!(normalize_numbers("十一"), "11");
    assert_eq!(normalize_numbers("一二三四"), "1234");
}
