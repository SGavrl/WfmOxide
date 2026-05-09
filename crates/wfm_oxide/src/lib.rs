use pyo3::prelude::*;
use numpy::{IntoPyArray, PyArray1};

pub mod dho;
pub mod mmap;
pub mod parser;
pub mod sample;
pub mod structs;

pub use mmap::{WfmFile, WfmHeader};

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
