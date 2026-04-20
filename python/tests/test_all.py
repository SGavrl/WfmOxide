import pytest
import wfm_oxide


def test_sum_as_string():
    assert wfm_oxide.sum_as_string(1, 1) == "2"
