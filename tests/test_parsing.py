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
    ("DS2072A-9.wfm", "2000")
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
    expected = 0.25 + 0.02 * (np.array([-10, 0, 10, 20], dtype=np.float32) - 5.0)
    
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
