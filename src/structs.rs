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
    pub display_delay: u32,
    pub display_address: u32,
    pub display_fine: u32,
    pub memory_address: u32,
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
    #[br(pad_before = 1)]
    pub time: TimeHeader1000E,
    // We ignore the rest of the header for now because it's not needed for basic parsing
    // and its size can vary or be complex.
}

impl WfmHeader1000E {
    pub fn ch1_skip(&self) -> usize { if self.roll_stop == 0 { 0 } else { (self.roll_stop + 2) as usize } }
    pub fn ch1_points(&self) -> usize { (self.ch1_memory_depth as usize).saturating_sub(self.ch1_skip()) }
    pub fn ch2_points(&self) -> usize {
        // ch2_memory_depth is usually the same as ch1_memory_depth for enabled channels
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

#[binread]
#[derive(Debug)]
#[br(little)]
pub struct TimeHeader1000E {
    pub scale_display: i64,
    pub offset_display: i64,
    pub sample_rate_hz: f32,
    pub scale_measured: i64,
    pub offset_measured: i64,
}

#[binread]
#[derive(Debug)]
#[br(little)]
pub struct TriggerHeader1000E {
    pub mode: u8,
    pub source: u8,
    pub coupling: u8,
    pub sweep: u8,
    #[br(pad_before = 1)]
    pub sens: f32,
    pub holdoff: f32,
    pub level: f32,
    pub direct: u8,
    pub pulse_type: u8,
    #[br(pad_before = 2)]
    pub pulse_width: f32,
    pub slope_type: u8,
}

// --- DS2000 ---
#[binread]
#[derive(Debug)]
#[br(little)]
pub struct WfmHeader2000 {
    #[br(pad_before = 8)]
    pub crc: u32,
    pub structure_size: u16,
    pub structure_version: u16,
    pub enabled_mask: u8, // channel_mask: ch4: b1, ch3: b1, ch2: b1, ch1: b1
    #[br(pad_before = 3)]
    #[br(count = 4)]
    pub channel_offsets: Vec<u32>,
    pub acquisition_mode: u16,
    pub average_time: u16,
    pub sample_mode: u16,
    #[br(pad_before = 2)]
    pub mem_depth: u32,
    pub sample_rate_hz: f32,
    // Add more if needed to reach channel headers at offset 132
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
    pub horiz_zoom_scale_factor: i32,
    pub horiz_zoom_position: f32,
    pub vert_zoom_scale_factor: f64,
    pub vert_zoom_position: f32,
    pub waveform_label: [u8; 32],
    pub n_frames: u32,
    pub wfm_header_size: u16,
}

pub struct TektronixHeader {
    pub static_info: TektronixStaticFileInfo,
    pub y_scale: f64,
    pub y_offset: f64,
    pub data_start_offset: u32,
    pub postcharge_start_offset: u32,
}
