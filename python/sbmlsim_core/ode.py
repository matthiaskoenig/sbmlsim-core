"""SBML equation analysis and standalone source exports from the shared Rust core.

The Python code handles source objects and files only. Mathematical analysis,
formatting, event semantics and all seven source templates run in Rust.
"""

from __future__ import annotations

import json
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from types import MappingProxyType
from typing import Any, Literal

from sbmlsim_core._core import _EquationSystem


@dataclass(frozen=True)
class OdeFormat:
    """A standalone executable or document export target."""

    name: str
    kind: Literal["code", "document"]
    suffixes: tuple[str, ...]
    options: Mapping[str, object]


ODE_FORMATS: Mapping[str, OdeFormat] = MappingProxyType(
    {
        name: OdeFormat(
            name,
            "code" if code else "document",
            suffixes,
            MappingProxyType(
                {"simulator": True} if code else {"standalone": True, "symbols": "id"}
            ),
        )
        for name, code, suffixes in [
            ("python", True, (".py",)),
            ("diffrax", True, (".py",)),
            ("julia", True, (".jl",)),
            ("r", True, (".R", ".r")),
            ("markdown", False, (".md",)),
            ("latex", False, (".tex",)),
            ("typst", False, (".typ",)),
        ]
    }
)


def _options(fmt: str, options: Mapping[str, object]) -> dict[str, object]:
    """Validate target options before entering the native generator."""
    if fmt not in ODE_FORMATS:
        raise ValueError(
            f"There is no format {fmt!r}, use one of {sorted(ODE_FORMATS)}."
        )
    defaults = ODE_FORMATS[fmt].options
    for key, value in options.items():
        if key not in defaults:
            raise ValueError(
                f"The format {fmt!r} has no option {key!r}, "
                f"its options are {sorted(defaults)}."
            )
        if type(value) is not type(defaults[key]):
            raise ValueError(
                f"The option {key!r} of the format {fmt!r} is a "
                f"{type(defaults[key]).__name__}, not {value!r}."
            )
    return {**defaults, **options}


_MAX_BYTES = 100 * 1024 * 1024


def _read_bytes(path: Path) -> bytes:
    """Bound host-side file reads before handing bytes to the native reader."""
    with path.open("rb") as stream:
        content = stream.read(_MAX_BYTES + 1)
    if len(content) > _MAX_BYTES:
        raise ValueError("The source exceeds 100 MB.")
    return content


class OdeSystem:
    """An immutable SBML equation source owned by Rust.

    Files, original XML text and bytes are accepted by :meth:`from_sbml`.
    Original bytes preserve floating-point values without an XML reserialization.
    Rendering never requires NumPy, SciPy, JAX, Julia or R at generation time.
    """

    def __init__(
        self,
        source: bytes,
        *,
        filename: str | None = None,
        documents: Mapping[str, str | bytes] | None = None,
        location: str = "./model.xml",
        max_expanded_elements: int = 10000,
    ) -> None:
        """Read a source through the pure Rust reader."""
        supplied = [
            (name, value.encode() if isinstance(value, str) else value)
            for name, value in (documents or {}).items()
        ]
        self._native = _EquationSystem(
            source, supplied, location, max_expanded_elements
        )
        self._filename = filename

    @classmethod
    def from_sbml(
        cls,
        source: Path | str | bytes,
        *,
        documents: Mapping[str, str | bytes] | None = None,
        location: str = "./model.xml",
        max_expanded_elements: int = 10000,
    ) -> OdeSystem:
        """Read original XML or an explicitly trusted file with local companions."""
        if isinstance(source, bytes):
            return cls(
                source,
                documents=documents,
                location=location,
                max_expanded_elements=max_expanded_elements,
            )
        if isinstance(source, str) and source.lstrip().startswith("<"):
            return cls(
                source.encode(),
                documents=documents,
                location=location,
                max_expanded_elements=max_expanded_elements,
            )
        if isinstance(source, (str, Path)):
            path = Path(source)
            if documents is None:
                system = cls.__new__(cls)
                system._native = _EquationSystem.from_path(
                    str(path), max_expanded_elements
                )
                system._filename = path.name
                return system
            return cls(
                _read_bytes(path),
                filename=path.name,
                documents=documents,
                location=location,
                max_expanded_elements=max_expanded_elements,
            )
        raise TypeError("Expected original SBML XML, bytes or a path.")

    @property
    def model_id(self) -> str | None:
        """The source identifier of the main model."""
        return self._native.model_id

    def render(self, fmt: str, **options: object) -> str:
        """Render standalone source code or a human-readable document."""
        options = _options(fmt, options)
        if self._filename is not None:
            options["source"] = self._filename
        return self._native.render(fmt, json.dumps(options))[1]

    def render_template(
        self, template: Path | str, fmt: str = "python", **options: object
    ) -> str:
        """Render a custom source template in Rust using the target's normal context."""
        options = _options(fmt, options)
        if self._filename is not None:
            options["source"] = self._filename
        source = Path(template).read_text(encoding="utf-8")
        return self._native.render_template(fmt, json.dumps(options), source)

    def typeset(self, links: Mapping[str, object] | None = None) -> dict[str, Any]:
        """Return linked LaTeX equations using opaque identifiers supplied by the caller.

        ``links`` contains ``symbols`` by SId, ``locals`` with reaction/index/link,
        and ``metadata`` with kind/metaid/link. No report object model is required.
        """
        return json.loads(self._native.typeset_json(json.dumps(links or {})))

    def write(
        self, path: Path | str, fmt: str | None = None, **options: object
    ) -> Path:
        """Write an export, inferring its format from the suffix when omitted."""
        path = Path(path)
        if fmt is None:
            fmt = next(
                (
                    name
                    for name, spec in ODE_FORMATS.items()
                    if path.suffix in spec.suffixes
                ),
                None,
            )
        if fmt is None:
            raise ValueError(f"No ODE format matches the suffix {path.suffix!r}.")
        path.write_text(self.render(fmt, **options), encoding="utf-8")
        return path


def render_ode(source: Path | str | bytes, fmt: str, **options: object) -> str:
    """Read an SBML source and render its equations through Rust."""
    return OdeSystem.from_sbml(source).render(fmt, **options)


# Compatibility spellings for callers migrating a former export namespace.
FORMATS = ODE_FORMATS
Format = OdeFormat


def render(system: OdeSystem, fmt: str, **options: object) -> str:
    """Render a shared equation source."""
    return system.render(fmt, **options)


def write(
    system: OdeSystem, path: Path | str, fmt: str | None = None, **options: object
) -> Path:
    """Write a shared equation source to a file."""
    return system.write(path, fmt, **options)


def render_template(
    system: OdeSystem, template: Path | str, fmt: str = "python", **options: object
) -> str:
    """Render a custom source template with the shared Rust generator."""
    return system.render_template(template, fmt, **options)
