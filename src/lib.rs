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

    #[getter]
    fn enabled_channels(&self) -> Vec<usize> {
        let mut enabled = Vec::new();
        match &self.inner.wfm_header {
            mmap::WfmHeader::Ds1000z(header) => {
                for i in 0..4 { if header.is_ch_enabled(i) { enabled.push(i + 1); } }
            },
            mmap::WfmHeader::Ds1000e(header) => {
                if header.channels[0].enabled_val != 0 { enabled.push(1); }
                if header.channels[1].enabled_val != 0 { enabled.push(2); }
            },
            mmap::WfmHeader::Ds2000(header) => {
                for i in 0..4 { if header.is_ch_enabled(i) { enabled.push(i + 1); } }
            },
            mmap::WfmHeader::Ds4000(header) => {
                for i in 0..4 { if header.is_ch_enabled(i) { enabled.push(i + 1); } }
            },
            mmap::WfmHeader::Tektronix(_) | mmap::WfmHeader::Isf(_) => {
                enabled.push(1);
            }
        }
        enabled
    }

    #[pyo3(signature = (channel, start=None, length=None))]
    fn get_channel_data<'py>(&self, py: Python<'py>, channel: usize, start: Option<usize>, length: Option<usize>) -> PyResult<Bound<'py, PyArray1<f32>>> {
        if channel < 1 || channel > 4 {
            return Err(pyo3::exceptions::PyValueError::new_err("Channel must be between 1 and 4"));
        }

        let result = py.allow_threads(|| {
            match &self.inner.wfm_header {
                mmap::WfmHeader::Ds1000z(header) => {
                    Parser::get_channel_data_1000z(&self.inner, header, channel - 1, start, length)
                },
                mmap::WfmHeader::Ds1000e(header) => {
                    Parser::get_channel_data_1000e(&self.inner, header, channel - 1, start, length)
                },
                mmap::WfmHeader::Ds2000(header) => {
                    Parser::get_channel_data_2000(&self.inner, header, channel - 1, start, length)
                },
                mmap::WfmHeader::Ds4000(header) => {
                    Parser::get_channel_data_4000(&self.inner, header, channel - 1, start, length)
                },
                mmap::WfmHeader::Tektronix(header) => {
                    Parser::get_channel_data_tektronix(&self.inner, header, channel - 1, start, length)
                },
                mmap::WfmHeader::Isf(header) => {
                    Parser::get_channel_data_isf(&self.inner, header, channel - 1, start, length)
                }
            }
        }).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;

        Ok(result.into_pyarray(py))
    }

    #[pyo3(signature = (start=None, length=None))]
    fn get_all_channels<'py>(&self, py: Python<'py>, start: Option<usize>, length: Option<usize>) -> PyResult<Vec<Option<Bound<'py, PyArray1<f32>>>>> {
        let results = py.allow_threads(|| {
            Parser::get_all_channels(&self.inner, start, length)
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
