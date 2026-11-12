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
    }
}
