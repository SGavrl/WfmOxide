from typing import Optional
import numpy as np
from numpy.typing import NDArray

__version__: str

class WfmOxide:
    def __init__(self, path: str) -> None: ...

    @property
    def model(self) -> str: ...
    @property
    def firmware(self) -> str: ...
    @property
    def enabled_channels(self) -> list[int]: ...
    @property
    def x_origin(self) -> Optional[float]: ...
    @property
    def x_increment(self) -> Optional[float]: ...
    @property
    def sample_rate(self) -> Optional[float]: ...

    def channel_metadata(self, channel: int) -> Optional[dict]: ...
    def get_time_axis(
        self,
        start: Optional[int] = None,
        length: Optional[int] = None,
    ) -> Optional[NDArray[np.float64]]: ...
    def get_channel_data(
        self,
        channel: int,
        start: Optional[int] = None,
        length: Optional[int] = None,
    ) -> NDArray[np.float32]: ...
    def get_all_channels(
        self,
        start: Optional[int] = None,
        length: Optional[int] = None,
    ) -> list[Optional[NDArray[np.float32]]]: ...
