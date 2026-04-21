use pyo3::prelude::*;
use numpy::{IntoPyArray, PyArray1};

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

    fn get_channel_data<'py>(&self, py: Python<'py>, channel: usize) -> PyResult<Bound<'py, PyArray1<f32>>> {
        if channel < 1 || channel > 4 {
            return Err(pyo3::exceptions::PyValueError::new_err("Channel must be between 1 and 4"));
        }

        let result = py.allow_threads(|| {
            match &self.inner.wfm_header {
                mmap::WfmHeader::Ds1000z(header) => {
                    Parser::get_channel_data_1000z(&self.inner, header, channel - 1)
                },
                mmap::WfmHeader::Ds1000e(header) => {
                    Parser::get_channel_data_1000e(&self.inner, header, channel - 1)
                },
                mmap::WfmHeader::Ds2000(header) => {
                    Parser::get_channel_data_2000(&self.inner, header, channel - 1)
                },
                mmap::WfmHeader::Tektronix(header) => {
                    Parser::get_channel_data_tektronix(&self.inner, header, channel - 1)
                },
                mmap::WfmHeader::Isf(header) => {
                    Parser::get_channel_data_isf(&self.inner, header, channel - 1)
                }
            }
        }).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;

        Ok(result.into_pyarray(py))
    }

    fn get_all_channels<'py>(&self, py: Python<'py>) -> PyResult<Vec<Option<Bound<'py, PyArray1<f32>>>>> {
        let results = py.allow_threads(|| {
            Parser::get_all_channels(&self.inner)
        }).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;

        let py_results = results.into_iter().map(|opt_vec| {
            opt_vec.map(|vec| vec.into_pyarray(py))
        }).collect();

        Ok(py_results)
    }
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<WfmOxide>()? ;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
