import os
import pytest
import numpy as np
import RigolWFM.wfm as rigol
from wfm_oxide import WfmOxide

@pytest.mark.parametrize("filename, model_id", [
    ("DS1074Z-C.wfm", "1000Z"),
    ("DS1054Z-ch1SquareCH2Uart.wfm", "1000Z"),
    ("DS1102E-F.wfm", "E"),
    ("DS1000E-B.wfm", "E"),
    ("DS2000-A.wfm", "2000"),
    ("DS2072A-9.wfm", "2000"),
    ("DS4024-B.wfm", "4000"),
    ("DHO1074.wfm", "DHO1000"),
    ("DHO824-ch1.wfm", "DHO800"),
    ("DHO824-ch12.wfm", "DHO800"),
    ("DHO824-ch1234.wfm", "DHO800"),
])
def test_correctness(filename, model_id):
    path = os.path.join("test_data", filename)
    
    # Parse with RigolWFM (the reference)
    w_ref = rigol.Wfm.from_file(path, model_id)
    
    # Parse with wfm_oxide
    w_oxide = WfmOxide(path)
    
    # Compare model (RigolWFM might have different names, but oxide captures raw string)
    if model_id == "1000Z":
        assert w_oxide.model == w_ref.header_name
    
    # Compare enabled channels
    for ch_idx in range(1, 5):
        # Find the reference channel by its channel_number
        ch_ref = next((c for c in w_ref.channels if c.channel_number == ch_idx), None)
        
        if ch_ref is not None:
            # Get voltage from both
            volts_ref = ch_ref.volts
            volts_oxide = w_oxide.get_channel_data(ch_idx)
            
            # Check length
            assert len(volts_oxide) == len(volts_ref)
            
            # Check values
            # Using slightly higher tolerance for E series as math might differ slightly
            np.testing.assert_allclose(volts_oxide, volts_ref, rtol=1e-3, atol=1e-5)
        else:
            # Should raise error for disabled channels in oxide
            with pytest.raises(ValueError):
                w_oxide.get_channel_data(ch_idx)

def test_tektronix_002():
    path = "test_data/tek_002.wfm"
    w_oxide = WfmOxide(path)
    assert w_oxide.model == "Tektronix"
    
    volts = w_oxide.get_channel_data(1)
    expected = 0.02 * np.array([-8, -1, 0, 7, 12], dtype=np.float32) - 1.0
    
    np.testing.assert_allclose(volts, expected, rtol=1e-5, atol=1e-5)

def test_tektronix_003():
    path = "test_data/tek_003.wfm"
    w_oxide = WfmOxide(path)
    assert w_oxide.model == "Tektronix"
    
    volts = w_oxide.get_channel_data(1)
    expected = 0.05 * np.array([-10, -5, 0, 5, 10], dtype=np.float32) + 0.25
    
    np.testing.assert_allclose(volts, expected, rtol=1e-5, atol=1e-5)

def test_tektronix_isf_16bit():
    path = "test_data/tek_synth.isf"
    w_oxide = WfmOxide(path)
    assert w_oxide.model == "Tektronix ISF"
    
    volts = w_oxide.get_channel_data(1)
    expected = 0.25 + 0.02 * (np.array([-10, 0, 10, 20], dtype=np.float32) - 5.0)
    
    np.testing.assert_allclose(volts, expected, rtol=1e-5, atol=1e-5)

def test_tektronix_isf_8bit():
    path = "test_data/tek_synth_8bit.isf"
    w_oxide = WfmOxide(path)
    assert w_oxide.model == "Tektronix ISF"
    
    volts = w_oxide.get_channel_data(1)
    expected = 0.0 + 0.02 * (np.array([-10, 0, 10, 20], dtype=np.float32) - 0.0)
    
    np.testing.assert_allclose(volts, expected, rtol=1e-5, atol=1e-5)

def test_get_all_channels():
    path = "test_data/DS1054Z-ch1SquareCH2Uart.wfm"
    w_oxide = WfmOxide(path)
    all_ch = w_oxide.get_all_channels()
    
    # This specific file has CH1 and CH2 enabled
    assert all_ch[0] is not None
    assert all_ch[1] is not None
    assert all_ch[2] is None
    assert all_ch[3] is None
    
    assert len(all_ch[0]) == 60256
    assert len(all_ch[1]) == 60256

def test_slicing():
    path = "test_data/DS1054Z-ch1SquareCH2Uart.wfm"
    w_oxide = WfmOxide(path)
    
    # Read full, then read slices
    full = w_oxide.get_channel_data(1)
    
    start = 100
    length = 50
    slice_data = w_oxide.get_channel_data(1, start=start, length=length)
    
    assert len(slice_data) == length
    np.testing.assert_allclose(slice_data, full[start:start+length])
    
    # Test get_all_channels slice
    all_ch = w_oxide.get_all_channels(start=start, length=length)
    assert len(all_ch[0]) == length
    np.testing.assert_allclose(all_ch[0], slice_data)

def test_enabled_channels():
    path = "test_data/DS1054Z-ch1SquareCH2Uart.wfm"
    w_oxide = WfmOxide(path)
    assert w_oxide.enabled_channels == [1, 2]


def test_no_enabled_channels_does_not_panic():
    # Regression: time_axis() previously divided by zero on this file because
    # stride() is 0 when no channels are enabled, which propagated as a
    # PanicException through the Python bindings.
    w = WfmOxide("test_data/DS1074Z-C.wfm")
    assert w.enabled_channels == []
    assert w.x_origin is None
    assert w.x_increment is None
    assert w.sample_rate is None
    assert w.get_time_axis() is None


def test_empty_file_is_rejected_cleanly(tmp_path):
    # Regression: an empty mmap previously panicked at &mmap[0..4].
    p = tmp_path / "empty.wfm"
    p.write_bytes(b"")
    with pytest.raises(OSError):
        WfmOxide(str(p))


def test_short_file_is_rejected_cleanly(tmp_path):
    # 3 bytes can't hold any known header.
    p = tmp_path / "short.wfm"
    p.write_bytes(b"\xde\xad\xbe")
    with pytest.raises(OSError):
        WfmOxide(str(p))


def test_truncated_ds1000z_decode_errors_cleanly(tmp_path):
    # Header parses but the sample payload claims more bytes than the file
    # has. Previously panicked at &mmap[data_start..]; now must raise.
    with open("test_data/DS1054Z-ch1SquareCH2Uart.wfm", "rb") as f:
        header_only = f.read(1024)
    p = tmp_path / "trunc.wfm"
    p.write_bytes(header_only)

    w = WfmOxide(str(p))
    with pytest.raises(ValueError, match="overruns file"):
        w.get_channel_data(1, length=5)


def test_garbage_ds1000e_decode_errors_cleanly(tmp_path):
    # Magic looks like DS1000E so header is parsed; channel scale/depth are
    # garbage. Previously panicked when the decode slice ran past EOF.
    body = b"\xa5\xa5\x00\x00" + bytes(range(256)) * 4
    p = tmp_path / "fake.wfm"
    p.write_bytes(body)

    w = WfmOxide(str(p))
    with pytest.raises(ValueError, match="overruns file"):
        w.get_channel_data(1, length=5)


def test_ds1000e_with_roll_stop(tmp_path):
    """Coverage: DS1000E with roll_stop > 0 (none of the committed fixtures
    set it). Synthesize a file by patching the roll_stop field of a real
    capture, then cross-check oxide vs RigolWFM."""
    import struct
    src = open("test_data/DS1000E-B.wfm", "rb").read()
    data = bytearray(src)
    data[20:24] = struct.pack("<I", 5)  # roll_stop offset is 20
    p = tmp_path / "rolled.wfm"
    p.write_bytes(bytes(data))

    w = WfmOxide(str(p))
    ref = rigol.Wfm.from_file(str(p), "E")
    expected = 8192 - (5 + 2)  # ch?_points = ch1_memory_depth - (roll_stop + 2)

    for ch in (1, 2):
        ch_ref = next(c for c in ref.channels if c.channel_number == ch)
        v = w.get_channel_data(ch)
        assert len(v) == expected
        assert len(ch_ref.volts) == expected
        np.testing.assert_allclose(v, ch_ref.volts, rtol=1e-3, atol=1e-5)


def _build_keysight_bin(channels, x_increment, x_origin, model="DSO-X 2024A", cookie=b"AG"):
    """Build a valid Keysight/Agilent InfiniiVision .bin in memory.

    Each entry in ``channels`` is a (name, numpy float32 array) tuple. Returns
    raw bytes ready to write to disk. Layout follows the public Keysight
    DSOX programmer's guide.
    """
    import struct
    import numpy as np

    parts = []

    # Waveform records first, into a buffer
    wf_blocks = b""
    for name, samples in channels:
        samples = np.asarray(samples, dtype=np.float32)
        n_points = len(samples)

        # Canonical 140-byte waveform header. Layout follows the public
        # Agilent/Keysight ".bin" file Programmer's Reference. Fields the
        # parser doesn't care about can be zero, but the *length* must match
        # the hdr_size field we declare.
        wf_hdr = struct.pack(
            "<i i i i i f d d d i i 16s 16s 24s 16s B B H I I",
            140,                         # hdr_size
            1,                           # wf_type = normal
            1,                           # n_buffers
            n_points,                    # n_points
            0,                           # count
            float(x_increment * n_points),  # x_disp_range
            float(x_origin),             # x_disp_origin
            float(x_increment),          # x_increment
            float(x_origin),             # x_origin
            2,                           # x_units = seconds
            1,                           # y_units = volts
            b"2026-05-16",               # date
            b"12:00:00",                 # time
            model.encode("ascii"),       # frame_model
            name.encode("ascii"),        # channel_name
            0,                           # acq_mode
            100,                         # completion %
            0,                           # x_units_subtype
            0,                           # segment_index
            0,                           # segment_count
        )
        assert len(wf_hdr) == 140, len(wf_hdr)
        # 12-byte data header + payload
        payload = samples.tobytes()
        data_hdr = struct.pack("<i h h i", 12, 1, 4, len(payload))
        wf_blocks += wf_hdr + data_hdr + payload

    file_size = 12 + len(wf_blocks)
    file_hdr = cookie + b"10" + struct.pack("<I i", file_size, len(channels))
    return file_hdr + wf_blocks


def test_keysight_bin_roundtrip(tmp_path):
    """Keysight format round-trip: build a synthetic .bin with known float32
    voltages, decode through oxide, and assert exact equality."""
    import numpy as np

    ch1 = np.linspace(-1.5, 1.5, 100, dtype=np.float32)
    ch2 = np.sin(np.linspace(0, 4 * np.pi, 100)).astype(np.float32)

    p = tmp_path / "synth.bin"
    p.write_bytes(_build_keysight_bin(
        channels=[("1", ch1), ("2", ch2)],
        x_increment=2e-9,
        x_origin=-1e-7,
        model="DSOX-3024A",
    ))

    w = WfmOxide(str(p))
    assert w.model == "DSOX-3024A"
    assert "Agilent/Keysight" in w.firmware
    assert w.enabled_channels == [1, 2]
    assert abs(w.sample_rate - 0.5e9) < 1.0
    assert abs(w.x_origin - (-1e-7)) < 1e-15
    assert abs(w.x_increment - 2e-9) < 1e-15

    v1 = w.get_channel_data(1)
    v2 = w.get_channel_data(2)
    np.testing.assert_array_equal(v1, ch1)
    np.testing.assert_array_equal(v2, ch2)

    # Slice + length
    v1_slice = w.get_channel_data(1, start=10, length=5)
    np.testing.assert_array_equal(v1_slice, ch1[10:15])

    # Time axis math
    t = w.get_time_axis()
    assert len(t) == 100
    assert abs(t[0] - (-1e-7)) < 1e-15
    assert abs(t[-1] - (-1e-7 + 99 * 2e-9)) < 1e-15

    # All channels
    all_ch = w.get_all_channels()
    assert all_ch[0] is not None and all_ch[1] is not None
    np.testing.assert_array_equal(all_ch[0], ch1)
    np.testing.assert_array_equal(all_ch[1], ch2)

    # No vertical scale/offset/coupling in this format
    assert w.channel_metadata(1) is None


def test_keysight_rg_cookie(tmp_path):
    """The 'RG' cookie variant (used by some Rigol exports) is also accepted."""
    import numpy as np
    ch = np.array([0.1, 0.2, 0.3], dtype=np.float32)
    p = tmp_path / "rg.bin"
    p.write_bytes(_build_keysight_bin(
        channels=[("1", ch)],
        x_increment=1e-6,
        x_origin=0.0,
        cookie=b"RG",
    ))
    w = WfmOxide(str(p))
    assert "Rigol" in w.firmware
    np.testing.assert_array_equal(w.get_channel_data(1), ch)


def test_keysight_invalid_cookie_falls_through(tmp_path):
    """A file starting with random bytes is not detected as Keysight; the
    fallthrough Rigol-magic check then rejects it cleanly."""
    p = tmp_path / "no.bin"
    p.write_bytes(b"XY10" + b"\x00" * 100)
    with pytest.raises(OSError):
        WfmOxide(str(p))


def test_isf_lsb_byte_order(tmp_path):
    """Coverage: ISF LSB (little-endian) byte order. All committed ISF
    fixtures are MSB. Synthesize a LSB variant of tek_synth.isf and confirm
    it decodes to the same samples as the MSB original."""
    import struct
    src = open("test_data/tek_synth.isf", "rb").read()
    curv = src.find(b":CURV")
    hash_pos = src.find(b"#", curv)
    n_digits = int(chr(src[hash_pos + 1]))
    data_off = hash_pos + 2 + n_digits

    header = src[:data_off].decode("latin-1").replace("BYT_O MSB", "BYT_O LSB")
    data_be = src[data_off:]
    n = len(data_be) // 2
    samples = struct.unpack(">" + "h" * n, data_be[:n * 2])
    data_le = struct.pack("<" + "h" * n, *samples)

    p = tmp_path / "isf_lsb.isf"
    p.write_bytes(header.encode("latin-1") + data_le)

    ref = WfmOxide("test_data/tek_synth.isf").get_channel_data(1)
    new = WfmOxide(str(p)).get_channel_data(1)
    np.testing.assert_array_equal(new, ref)
