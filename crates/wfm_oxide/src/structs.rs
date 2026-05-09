#![allow(dead_code)]
use binrw::binread;

#[binread]
#[derive(Debug)]
#[br(little)]
pub struct FileHeader {
    pub magic: [u8; 4],
    pub magic2: u16,
    pub structure_size: u16,
    #[br(map = |s: [u8; 20]| String::from_utf8_lossy(&s).trim_end_matches('\0').to_string())]
    pub model_number: String,
    #[br(map = |s: [u8; 20]| String::from_utf8_lossy(&s).trim_end_matches('\0').to_string())]
    pub firmware_version: String,
    pub block: [u8; 2],
    pub file_version: u16,
}

// --- DS1000Z ---
#[binread]
#[derive(Debug)]
#[br(little)]
pub struct WfmHeader1000Z {
    pub picoseconds_per_division: u64,
    pub picoseconds_offset: i64,
    pub crc: u32,
    pub structure_size: [u8; 2],
    pub structure_version: u16,
    pub flags: u8,
    #[br(pad_before = 3)]
    pub ch1_file_offset: u32,
    pub ch2_file_offset: u32,
    pub ch3_file_offset: u32,
    pub ch4_file_offset: u32,
    pub la_offset: u32,
    pub acq_mode: u8,
    pub average_time: u8,
    #[br(pad_before = 1)]
    pub time_mode: u8,
    pub memory_depth: u32,
    pub sample_rate_ghz: f32,
    #[br(count = 4)]
    pub channels: Vec<ChannelHeader1000Z>,
    #[br(pad_before = 12)]
    pub setup_size: u32,
    pub setup_offset: u32,
    pub horizontal_size: u32,
    pub horizontal_offset: u32,
}

#[binread]
#[derive(Debug)]
#[br(little)]
pub struct ChannelHeader1000Z {
    pub enabled_val: u8,
    pub coupling: u8,
    pub bandwidth_limit: u8,
    pub probe_type: u8,
    pub probe_ratio: u8,
    #[br(pad_before = 3)]
    pub scale: f32,
    pub shift: f32,
    pub inverted_val: u8,
    pub unit: u8,
    #[br(pad_before = 10)]
    pub unknown: (),
}

impl WfmHeader1000Z {
    pub fn is_ch_enabled(&self, ch: usize) -> bool { (self.flags >> ch) & 1 != 0 }
    pub fn enabled_channels_count(&self) -> usize { (0..4).filter(|&i| self.is_ch_enabled(i)).count() }
    pub fn stride(&self) -> usize {
        let count = self.enabled_channels_count();
        if count == 3 { 4 } else { count }
    }
    pub fn points(&self) -> u32 { self.memory_depth / self.stride() as u32 }
}

// --- DS1000E ---
#[binread]
#[derive(Debug)]
#[br(little)]
pub struct WfmHeader1000E {
    pub magic: [u8; 4],
    pub unknown_1: u16,
    #[br(pad_before = 10)]
    pub adc_mode: u8,
    #[br(pad_before = 3)]
    pub roll_stop: u32,
    #[br(pad_before = 4)]
    pub ch1_memory_depth: u32,
    pub active_channel: u8,
    #[br(pad_before = 1)]
    pub channels: [ChannelHeader1000E; 2],
    pub time_offset: u8,
}

impl WfmHeader1000E {
    pub fn ch1_skip(&self) -> usize { if self.roll_stop == 0 { 0 } else { (self.roll_stop + 2) as usize } }
    pub fn ch1_points(&self) -> usize { (self.ch1_memory_depth as usize).saturating_sub(self.ch1_skip()) }
    pub fn ch2_points(&self) -> usize {
        let ch2_mem_depth = self.ch1_memory_depth; 
        (ch2_mem_depth as usize).saturating_sub(self.ch1_skip()) 
    }
}
#[binread]
#[derive(Debug)]
#[br(little)]
pub struct ChannelHeader1000E {
    pub unknown_0: u16,
    pub scale_display: i32,
    pub shift_display: i16,
    pub unknown_1: u8,
    pub unknown_2: u8,
    pub probe_value: f32,
    pub invert_disp_val: u8,
    pub enabled_val: u8,
    pub inverted_m_val: u8,
    pub unknown_3: u8,
    pub scale_measured: i32,
    pub shift_measured: i16,
}

// --- DS2000 ---
#[binread]
#[derive(Debug)]
#[br(little)]
pub struct FileHeader2000 {
    pub magic: [u8; 4],
    #[br(map = |s: [u8; 20]| String::from_utf8_lossy(&s).trim_end_matches('\0').to_string())]
    pub model_number: String,
    #[br(map = |s: [u8; 20]| String::from_utf8_lossy(&s).trim_end_matches('\0').to_string())]
    pub firmware_version: String,
}

#[binread]
#[derive(Debug)]
#[br(little)]
pub struct WfmHeader2000 {
    pub crc: u32,
    pub structure_size: u16,
    pub structure_version: u16,
    pub enabled_mask: u16, // channel_mask
    pub extra_1a: [u8; 2],
    pub channel_offsets: [u32; 4],
    pub acquisition_mode: u16,
    pub average_time: u16,
    pub sample_mode: u16,
    pub extra_1b: [u8; 2],
    pub mem_depth: u32,
    pub sample_rate_hz: f32,
    pub extra_1c: [u8; 2],
    pub time_mode: u16,
    pub time_scale_ps: u64,
    pub time_offset_ps: i64,
    pub channels: [ChannelHeader2000; 4],
    pub len_setup: u32,
    pub setup_offset: u32,
    pub wfm_offset: u32,
    pub storage_depth: u32,
    pub z_pt_offset: u32,
    pub wfm_len: u32,
}

impl WfmHeader2000 {
    pub fn is_ch_enabled(&self, ch: usize) -> bool { self.channels[ch].is_enabled() }
    
    pub fn interwoven(&self) -> bool { (self.enabled_mask >> 8) & 1 != 0 }
    
    pub fn raw_depth(&self) -> usize {
        let len = self.wfm_len as usize;
        if self.interwoven() { len / 2 } else { len }
    }
}

#[binread]
#[derive(Debug)]
#[br(little)]
pub struct ChannelHeader2000 {
    pub enabled_temp: u8,
    pub coupling_raw: u8,
    pub bandwidth_limit: u8,
    pub probe_type: u8,
    pub probe_ratio_raw: u8,
    pub probe_diff: u8,
    pub probe_signal: u8,
    pub probe_impedance_raw: u8,
    pub volt_per_division: f32,
    pub volt_offset: f32,
    pub inverted_temp: u8,
    pub unit_temp: u8,
    pub filter_enabled: u8,
    pub filter_type: u8,
    pub filter_high: u32,
    pub filter_low: u32,
}

impl ChannelHeader2000 {
    pub fn is_enabled(&self) -> bool { self.enabled_temp != 0 }
    pub fn is_inverted(&self) -> bool {
        let legacy_vertical = self.enabled_temp == 1;
        let inv_actual = if legacy_vertical { self.inverted_temp } else { self.unit_temp };
        inv_actual == 1
    }
    pub fn volt_signed(&self) -> f32 {
        if self.is_inverted() { -self.volt_per_division } else { self.volt_per_division }
    }
    pub fn volt_scale(&self) -> f32 { self.volt_signed() / 25.0 }
}

// --- DS4000 ---
#[binread]
#[derive(Debug)]
#[br(little)]
pub struct WfmHeader4000 {
    pub unknown_1: [u32; 5],
    pub enabled_mask: u8,
    pub unknown_2: [u8; 3],
    pub channel_offsets: [u32; 4],
    pub unknown_3: u32,
    pub unknown_4: u32,
    pub unknown_5: u32,
    pub mem_depth_1: u32,
    pub sample_rate_hz: f32,
    pub unknown_8: u32,
    pub time_per_div_ps: u64,
    pub unknown_9: [u32; 2],
    pub channels: [ChannelHeader4000; 4],
    pub unknown_33: [u32; 6],
    pub mem_depth_2: u32,
    pub unknown_37: u32,
    pub mem_depth: u32,
}

impl WfmHeader4000 {
    pub fn is_ch_enabled(&self, ch: usize) -> bool { (self.enabled_mask >> ch) & 1 != 0 }
}

#[binread]
#[derive(Debug)]
#[br(little)]
pub struct ChannelHeader4000 {
    pub enabled_val: u8,
    pub coupling: u8,
    pub bandwidth_limit: u8,
    pub probe_type: u8,
    pub probe_ratio: u8,
    pub probe_diff: u8,
    pub probe_signal: u8,
    pub probe_impedance: u8,
    pub volt_per_division: f32,
    pub volt_offset: f32,
    pub inverted_val: u8,
    pub unit: u8,
    pub filter_enabled: u8,
    pub filter_type: u8,
    pub filter_high: u32,
    pub filter_low: u32,
}

impl ChannelHeader4000 {
    pub fn is_enabled(&self) -> bool { self.enabled_val != 0 }
    pub fn is_inverted(&self) -> bool { self.inverted_val != 0 }
    pub fn volt_signed(&self) -> f32 {
        if self.is_inverted() { -self.volt_per_division } else { self.volt_per_division }
    }
}

// --- Tektronix ---
#[binread]
#[derive(Debug)]
pub struct TektronixStaticFileInfo {
    pub byte_order: u16,
    #[br(map = |s: [u8; 8]| String::from_utf8_lossy(&s).trim_end_matches('\0').to_string())]
    pub version_number: String,
    pub num_digits_byte_count: u8,
    pub num_bytes_to_eof: i32,
    pub num_bytes_per_point: u8,
    pub byte_offset_to_curve_buffer: i32,
}

pub struct TektronixHeader {
    pub static_info: TektronixStaticFileInfo,
    pub y_scale: f64,
    pub y_offset: f64,
    pub data_start_offset: u32,
    pub postcharge_start_offset: u32,
}

// --- Tektronix ISF ---
#[derive(Debug)]
pub struct IsfHeader {
    pub byt_nr: u8,
    pub byt_or: String,
    pub nr_pt: u32,
    pub ymult: f32,
    pub yoff: f32,
    pub yzero: f32,
    pub data_offset: usize,
}
