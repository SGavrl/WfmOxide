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
        
    @property
    def enabled_channels(self) -> list[int]:
        """Returns a list of integer channel numbers that were enabled during capture."""
        return self._inner.enabled_channels

    def get_channel_data(self, channel: int, start: int = None, length: int = None):
        """
        Returns the voltage data for the specified channel as a NumPy array.
        
        Args:
            channel: The channel number (1-4).
            start: Optional. The starting index for data slicing.
            length: Optional. The number of points to extract.
        """
        return self._inner.get_channel_data(channel, start, length)

    def get_all_channels(self, start: int = None, length: int = None):
        """
        Returns a list of NumPy arrays for all channels. 
        Channels that are not enabled will be None.

        Args:
            start: Optional. The starting index for data slicing.
            length: Optional. The number of points to extract.
        """
        return self._inner.get_all_channels(start, length)
