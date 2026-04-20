import os
import RigolWFM.wfm as rigol
from wfm_oxide import WfmOxide

def test_benchmark_rigolwfm(benchmark):
    path = "test_data/DS1074Z-C.wfm"
    def load():
        w = rigol.Wfm.from_file(path, '1000Z')
        for ch in w.channels:
            if ch.enabled:
                _ = ch.volts
    benchmark(load)

def test_benchmark_wfm_oxide(benchmark):
    path = "test_data/DS1074Z-C.wfm"
    def load():
        w = WfmOxide(path)
        for i in range(1, 5):
            try:
                _ = w.get_channel_data(i)
            except ValueError:
                pass
    benchmark(load)
