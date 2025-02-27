from typing import TypeVar, Generic, Union, Optional, Protocol, Tuple, List, Any, Self
from types import TracebackType
from enum import Flag, Enum, auto
from dataclasses import dataclass
from abc import abstractmethod
import weakref

from ..types import Result, Ok, Err, Some


class LanguageCode(Enum):
    """
    ISO 639-3
    """
    AFR = 0
    ARA = 1
    AZE = 2
    BEL = 3
    BEN = 4
    BOS = 5
    BUL = 6
    CAT = 7
    CES = 8
    CYM = 9
    DAN = 10
    DEU = 11
    ELL = 12
    ENG = 13
    EPO = 14
    EST = 15
    EUS = 16
    FAS = 17
    FIN = 18
    FRA = 19
    GLE = 20
    GUJ = 21
    HEB = 22
    HIN = 23
    HRV = 24
    HUN = 25
    HYE = 26
    IND = 27
    ISL = 28
    ITA = 29
    JPN = 30
    KAT = 31
    KAZ = 32
    KOR = 33
    LAT = 34
    LAV = 35
    LIT = 36
    LUG = 37
    MAR = 38
    MKD = 39
    MON = 40
    MRI = 41
    MSA = 42
    NLD = 43
    NNO = 44
    NOB = 45
    PAN = 46
    POL = 47
    POR = 48
    RON = 49
    RUS = 50
    SLK = 51
    SLV = 52
    SNA = 53
    SOM = 54
    SOT = 55
    SPA = 56
    SRP = 57
    SQI = 58
    SWA = 59
    SWE = 60
    TAM = 61
    TEL = 62
    TGL = 63
    THA = 64
    TSN = 65
    TSO = 66
    TUR = 67
    UKR = 68
    URD = 69
    VIE = 70
    XHO = 71
    YOR = 72
    ZHO = 73
    ZUL = 74

@dataclass
class SelectLanguageRequest:
    """
    Select the detected language for the provided input based on the list of possible languages.
    If no language matches, None is returned.
    
    text: Text input
    languages: All languages that should be considered during detection.
    """
    text: str
    languages: List[LanguageCode]


def select_language(request: List[SelectLanguageRequest]) -> List[Optional[LanguageCode]]:
    raise NotImplementedError

