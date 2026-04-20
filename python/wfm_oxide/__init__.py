from ._core import WfmOxide as _WfmOxide

class WfmOxide:
    """A high-performance Rigol .wfm file parser."""
    
    def __init__(self, path: str):
        self._inner = _WfmOxide(path)
        
    @property
    def model(self) -> str:
        """The oscilloscope model number."""
        return self._inner.model
        
    @property
    def firmware(self) -> str:
        """The oscilloscope firmware version."""
        return self._inner.firmware
        
    def get_channel_data(self, channel: int):
        """
        Returns the voltage data for the specified channel as a NumPy array.
        
        Args:
            channel: The channel number (1-4).
        """
        return self._inner.get_channel_data(channel)
