from typing import TypeVar, Generic, Union, Optional, Protocol, Tuple, List, Any, Self
from types import TracebackType
from enum import Flag, Enum, auto
from dataclasses import dataclass
from abc import abstractmethod
import weakref

from ..types import Result, Ok, Err, Some
from ..exports import stream_skill_handler

class StreamSkillHandler(Protocol):

    @abstractmethod
    def run(self, input: bytes) -> None:
        """
        Raises: `stream_skill.types.Err(stream_skill.imports.stream_skill_handler.Error)`
        """
        raise NotImplementedError

    @abstractmethod
    def metadata(self) -> stream_skill_handler.StreamSkillMetadata:
        raise NotImplementedError


