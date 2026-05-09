use pyo3::prelude::*;
use numpy::{IntoPyArray, PyArray1};

pub mod dho;
pub mod mmap;
pub mod parser;
pub mod sample;
pub mod structs;

pub use mmap::{TimeAxis, WfmFile, WfmHeader};

#[pyclass]
struct WfmOxide {
    inner: WfmFile,
}

#[pymethods]
impl WfmOxide {
    #[new]
    fn new(path: &str) -> PyResult<Self> {
        let inner = WfmFile::open(path).map_err(|e| {
            pyo3::exceptions::PyOSError::new_err(format!("Failed to open WFM file: {}", e))
        })?;
        Ok(WfmOxide { inner })
    }

    #[getter]
    fn model(&self) -> String {
        self.inner.model_number.clone()
    }

    #[getter]
    fn firmware(&self) -> String {
        self.inner.firmware_version.clone()
    }

    #[getter]
    fn enabled_channels(&self) -> Vec<usize> {
        self.inner.enabled_channels()
    }

    #[getter]
    fn x_origin(&self) -> Option<f64> {
        self.inner.time_axis().map(|t| t.x_origin)
    }

    #[getter]
    fn x_increment(&self) -> Option<f64> {
        self.inner.time_axis().map(|t| t.x_increment)
    }

    #[getter]
    fn sample_rate(&self) -> Option<f64> {
        self.inner.time_axis().map(|t| t.sample_rate())
    }

    #[pyo3(signature = (start=None, length=None))]
    fn get_time_axis<'py>(&self, py: Python<'py>, start: Option<usize>, length: Option<usize>) -> PyResult<Option<Bound<'py, PyArray1<f64>>>> {
        let Some(t) = self.inner.time_axis() else {
            return Ok(None);
        };
        let Some(channel) = self.inner.enabled_channels().first().copied() else {
            return Ok(None);
        };
        let total = self
            .inner
            .channel_sample_count(channel)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        let start = start.unwrap_or(0).min(total);
        let len = length.unwrap_or(total - start).min(total - start);

        let times: Vec<f64> = (0..len)
            .map(|i| t.x_origin + (start + i) as f64 * t.x_increment)
            .collect();
        Ok(Some(times.into_pyarray(py)))
    }

    #[pyo3(signature = (channel, start=None, length=None))]
    fn get_channel_data<'py>(&self, py: Python<'py>, channel: usize, start: Option<usize>, length: Option<usize>) -> PyResult<Bound<'py, PyArray1<f32>>> {
        let result = py
            .allow_threads(|| self.inner.extract_channel(channel, start, length))
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        Ok(result.into_pyarray(py))
    }

    #[pyo3(signature = (start=None, length=None))]
    fn get_all_channels<'py>(&self, py: Python<'py>, start: Option<usize>, length: Option<usize>) -> PyResult<Vec<Option<Bound<'py, PyArray1<f32>>>>> {
        let results = py
            .allow_threads(|| self.inner.extract_all_channels(start, length))
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;

        let py_results = results
            .into_iter()
            .map(|opt_vec| opt_vec.map(|vec| vec.into_pyarray(py)))
            .collect();

        Ok(py_results)
    }
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<WfmOxide>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
