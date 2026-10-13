//! Валидация показаний носимых датчиков (контракт: docs/ARCHITECTURE.md, разделы 2 и 10).
//! Таблица ниже должна совпадать с `_RANGES` в python/vitals/validation.py
//! (совпадение проверяет тест).

use numpy::{PyArray1, PyReadonlyArray1};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::pybacked::PyBackedStr;
use pyo3::types::PyDict;

/// Коды причин (раздел 10).
pub const OK: u8 = 0;
pub const MISSING: u8 = 1;
pub const OUT_OF_RANGE: u8 = 2;
pub const UNKNOWN_METRIC: u8 = 3;
pub const BAD_UNIT: u8 = 4;

/// (метрика, нижняя граница, верхняя граница, единица). Границы включительно.
pub const RANGES: [(&str, f64, f64, &str); 7] = [
    ("heart_rate", 25.0, 250.0, "bpm"),
    ("spo2", 50.0, 100.0, "%"),
    ("temperature", 30.0, 43.0, "C"),
    ("systolic_bp", 50.0, 260.0, "mmHg"),
    ("diastolic_bp", 30.0, 160.0, "mmHg"),
    ("resp_rate", 4.0, 60.0, "breaths/min"),
    ("steps", 0.0, 400.0, "count"),
];

fn lookup(metric: &str) -> Option<&'static (&'static str, f64, f64, &'static str)> {
    // 7 записей — линейный поиск быстрее хэш-таблицы
    RANGES.iter().find(|r| r.0 == metric)
}

/// Чистая функция валидации одной строки.
/// Приоритет при нескольких нарушениях: 3 (метрика) → 1 (пропуск) → 2 (диапазон) → 4 (единица).
/// `None` в метрике считается неизвестной метрикой, в единице — неверной единицей.
pub fn validate_row(metric: Option<&str>, value: f64, unit: Option<&str>) -> u8 {
    let Some(&(_, lo, hi, expected_unit)) = metric.and_then(lookup) else {
        return UNKNOWN_METRIC;
    };
    if value.is_nan() {
        return MISSING;
    }
    // inf тоже вне диапазона: сравнение ниже его отсекает
    if !(value >= lo && value <= hi) {
        return OUT_OF_RANGE;
    }
    // точное совпадение, без нормализации регистра ("BPM" — ошибка)
    if unit != Some(expected_unit) {
        return BAD_UNIT;
    }
    OK
}

fn validate_slices(
    metric: &[Option<PyBackedStr>],
    value: &[f64],
    unit: &[Option<PyBackedStr>],
) -> Vec<u8> {
    metric
        .iter()
        .zip(value)
        .zip(unit)
        .map(|((m, &v), u)| {
            validate_row(
                m.as_ref().map(|s| s.as_ref() as &str),
                v,
                u.as_ref().map(|s| s.as_ref() as &str),
            )
        })
        .collect()
}

/// validate_columns(metric, value, unit) -> numpy.ndarray[uint8]
#[pyfunction]
fn validate_columns<'py>(
    py: Python<'py>,
    metric: Vec<Option<PyBackedStr>>,
    value: PyReadonlyArray1<'py, f64>,
    unit: Vec<Option<PyBackedStr>>,
) -> PyResult<Bound<'py, PyArray1<u8>>> {
    let n = value.len()?;
    if metric.len() != n || unit.len() != n {
        return Err(PyValueError::new_err(format!(
            "длины входов не совпадают: metric={}, value={}, unit={}",
            metric.len(),
            n,
            unit.len()
        )));
    }
    // readonly view без копии; если массив не непрерывный — as_slice вернёт ошибку
    let value_slice = value.as_slice().map_err(|_| {
        PyValueError::new_err("value должен быть непрерывным одномерным массивом float64")
    })?;
    // PyBackedStr держит ссылку на строки Python и не требует GIL для чтения
    let codes = py.allow_threads(|| validate_slices(&metric, value_slice, &unit));
    Ok(PyArray1::from_vec(py, codes))
}

/// ranges() -> dict[str, (low, high, unit)]
#[pyfunction]
fn ranges(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let d = PyDict::new(py);
    for (m, lo, hi, u) in RANGES {
        d.set_item(m, (lo, hi, u))?;
    }
    Ok(d)
}

#[pymodule]
fn vitals_validate(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(validate_columns, m)?)?;
    m.add_function(wrap_pyfunction!(ranges, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_row() {
        assert_eq!(validate_row(Some("heart_rate"), 72.0, Some("bpm")), OK);
        assert_eq!(validate_row(Some("resp_rate"), 15.0, Some("breaths/min")), OK);
    }

    #[test]
    fn unknown_metric() {
        assert_eq!(validate_row(Some("glucose"), 5.0, Some("mmol")), UNKNOWN_METRIC);
        assert_eq!(validate_row(None, 5.0, Some("bpm")), UNKNOWN_METRIC);
        assert_eq!(validate_row(Some("HEART_RATE"), 72.0, Some("bpm")), UNKNOWN_METRIC);
    }

    #[test]
    fn missing_value() {
        assert_eq!(validate_row(Some("spo2"), f64::NAN, Some("%")), MISSING);
    }

    #[test]
    fn bad_unit() {
        assert_eq!(validate_row(Some("heart_rate"), 72.0, Some("BPM")), BAD_UNIT);
        assert_eq!(validate_row(Some("heart_rate"), 72.0, Some("")), BAD_UNIT);
        assert_eq!(validate_row(Some("heart_rate"), 72.0, None), BAD_UNIT);
        assert_eq!(validate_row(Some("heart_rate"), 72.0, Some("bpm ")), BAD_UNIT);
    }

    #[test]
    fn priority_order() {
        // 3 важнее всего
        assert_eq!(validate_row(Some("x"), f64::NAN, Some("")), UNKNOWN_METRIC);
        // 1 важнее 2 и 4
        assert_eq!(validate_row(Some("spo2"), f64::NAN, Some("BAD")), MISSING);
        // 2 важнее 4
        assert_eq!(validate_row(Some("spo2"), -1.0, Some("BAD")), OUT_OF_RANGE);
    }

    #[test]
    fn bounds_are_inclusive() {
        for (m, lo, hi, u) in RANGES {
            assert_eq!(validate_row(Some(m), lo, Some(u)), OK, "{m} нижняя граница");
            assert_eq!(validate_row(Some(m), hi, Some(u)), OK, "{m} верхняя граница");
            assert_eq!(validate_row(Some(m), lo - 1e-9, Some(u)), OUT_OF_RANGE, "{m} ниже");
            assert_eq!(validate_row(Some(m), hi + 1e-9, Some(u)), OUT_OF_RANGE, "{m} выше");
        }
    }

    #[test]
    fn infinities_out_of_range() {
        assert_eq!(validate_row(Some("steps"), f64::INFINITY, Some("count")), OUT_OF_RANGE);
        assert_eq!(validate_row(Some("steps"), f64::NEG_INFINITY, Some("count")), OUT_OF_RANGE);
    }

    #[test]
    fn spec_examples() {
        assert_eq!(validate_row(Some("heart_rate"), 999.0, Some("bpm")), OUT_OF_RANGE);
        assert_eq!(validate_row(Some("spo2"), -1.0, Some("%")), OUT_OF_RANGE);
        assert_eq!(validate_row(Some("steps"), 0.0, Some("count")), OK);
        assert_eq!(validate_row(Some("temperature"), 43.0, Some("C")), OK);
    }
}
