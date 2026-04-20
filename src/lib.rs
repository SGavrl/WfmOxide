use pyo3::prelude::*;
use numpy::PyArray1;

mod mmap;
mod parser;
mod structs;

use mmap::WfmFile;
use parser::Parser;

#[pyclass]
struct WfmOxide {
    inner: WfmFile,
}

#[pymethods]
impl WfmOxide {
    #[new]
    fn new(path: &str) -> PyResult<Self> {
        let inner = WfmFile::open(path).map_err(|e| {
            pyo3::exceptions::PyIOError::new_err(format!("Failed to open WFM file: {}", e))
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

    fn get_channel_data<'py>(&self, py: Python<'py>, channel: usize) -> PyResult<Bound<'py, PyArray1<f32>>> {
        if channel < 1 || channel > 4 {
            return Err(pyo3::exceptions::PyValueError::new_err("Channel must be between 1 and 4"));
        }
        
        match &self.inner.wfm_header {
            mmap::WfmHeader::Ds1000z(header) => {
                Parser::get_channel_data_1000z(py, &self.inner, header, channel - 1)
            },
            mmap::WfmHeader::Ds1000e(header) => {
                Parser::get_channel_data_1000e(py, &self.inner, header, channel - 1)
            },
            mmap::WfmHeader::Tektronix(header) => {
                Parser::get_channel_data_tektronix(py, &self.inner, header, channel - 1)
            }
        }
    }
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<WfmOxide>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
