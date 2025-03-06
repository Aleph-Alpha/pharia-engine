"""
Allows to send multiple message delta events
"""
from typing import TypeVar, Generic, Union, Optional, Protocol, Tuple, List, Any, Self
from types import TracebackType
from enum import Flag, Enum, auto
from dataclasses import dataclass
from abc import abstractmethod
import weakref

from ..types import Result, Ok, Err, Some


@dataclass
class MessageDelta:
    role: Optional[str]
    content: str


def write_stream_event(event: MessageDelta) -> None:
    raise NotImplementedError

