//! Форматирование времени для интерфейса (числа одинаковы во всех языках).

/// 75.4 → «1:15», 6612 → «1:50:12».
pub fn duration(secs: f64) -> String {
    let t = secs.max(0.0).round() as u64;
    let (h, m, s) = (t / 3600, t / 60 % 60, t % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// Время с миллисекундами: 75.4 → «1:15.400», 6612.05 → «1:50:12.050».
pub fn clock_ms(secs: f64) -> String {
    let ms = (secs.max(0.0) * 1000.0).round() as u64;
    let (t, frac) = (ms / 1000, ms % 1000);
    let (h, m, s) = (t / 3600, t / 60 % 60, t % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}.{frac:03}") } else { format!("{m}:{s:02}.{frac:03}") }
}

/// Подпись деления шкалы: при шаге меньше секунды — с десятыми.
pub fn tick(secs: f64, step: f64) -> String {
    if step >= 1.0 {
        return duration(secs);
    }
    let ds = (secs.max(0.0) * 10.0).round() as u64;
    format!("{}.{}", duration((ds / 10) as f64), ds % 10)
}

/// Число без лишних нулей: 29.97 → «29.97», 30.0 → «30», 1.25 → «1.25», 1.234 → «1.23».
pub fn number(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Дата по местному времени: секунды Unix → «2026-11-15».
pub fn date(unix: u64) -> String {
    let t = unix as libc::time_t;
    // SAFETY: localtime_r пишет только в переданную структуру.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return civil(unix as i64 / 86_400);
    }
    format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
}

/// Дни от 1970-01-01 → дата (UTC; запасной путь).
fn civil(days: i64) -> String {
    // Алгоритм Говарда Хиннанта (days_from_civil наоборот).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(duration(75.4), "1:15");
        assert_eq!(duration(6612.0), "1:50:12");
        assert_eq!(duration(0.2), "0:00");
        assert_eq!(clock_ms(75.4), "1:15.400");
        assert_eq!(clock_ms(6612.05), "1:50:12.050");
        assert_eq!(clock_ms(-1.0), "0:00.000");
        assert_eq!(tick(62.5, 0.5), "1:02.5");
        assert_eq!(tick(62.5, 5.0), "1:03");
        assert_eq!(number(29.97002997), "29.97");
        assert_eq!(number(30.0), "30");
        assert_eq!(number(1.5), "1.5");
        assert_eq!(civil(0), "1970-01-01");
        assert_eq!(civil(20_772), "2026-11-15");
        assert_eq!(civil(19_782), "2024-02-29");
        assert_eq!(date(1_763_200_000).len(), 10);
    }
}
