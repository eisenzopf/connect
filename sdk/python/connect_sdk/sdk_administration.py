"""Explicit trusted backend; Rust owns provisioning, current authority and status."""
from __future__ import annotations

from dataclasses import dataclass
import os
import re
from typing import Any, Callable, Generic, TypeVar, cast
from weakref import WeakKeyDictionary

from ._sdk_helper import _SdkHelper, _encode, _parse
from ._sdk_validation import _validate
from .client import Client, Operation, OperationKind, OperationReference, _KINDS, _PROOF, _bad, _object
from .sdk_types import DecisionReceipt, Json, MutationOptions, OperationId, OperationKey, RequestOptions, Scope, SdkError, freeze, identifier

T = TypeVar("T")
R = TypeVar("R")


@dataclass(frozen=True, repr=False)
class _Descriptor:
    value: Any
    reference: Any
    kind: str


class TrustedProvisioningCommand:
    """Immutable normalized command metadata, never a grant or implicit submission."""
    __slots__ = ("__weakref__",)

    def __new__(cls):
        raise SdkError("validation", "not-submitted")

    @property
    def value(self) -> Json:
        return self._saved().value

    @property
    def operation_id(self) -> OperationId:
        return OperationId(self._saved().reference["operation_id"])

    def reference_json(self) -> Json:
        return _encode(self._saved().reference, 4096)[0]

    def to_json(self) -> Json:
        """Export explicit private command intent, including sensitive fields."""
        return _encode(self.value, 65536)[0]

    def _saved(self) -> _Descriptor:
        saved = _commands.get(self)
        if saved is None:
            raise SdkError("validation", "not-submitted")
        return saved


_commands: WeakKeyDictionary[TrustedProvisioningCommand, _Descriptor] = WeakKeyDictionary()
_administrative_kinds: dict[str, OperationKind[Json]] = {}


def _json(value: Any, client: Client, key: OperationKey, saved: Json) -> Json:
    # The original Rust administrative kind validates its ProvisioningOutcome.
    # This facade exposes the bounded immutable Json result, not a second schema.
    return freeze(value)


def _select(command: str, name: str) -> OperationKind[Json] | OperationKind[DecisionReceipt]:
    if name == "operation_cancel" and command == "operation.cancel":
        return _KINDS.operation_cancel
    if name == "operation_reconcile" and command == "operation.reconcile":
        return _KINDS.operation_reconcile
    if name != "administrative" or command in ("operation.cancel", "operation.reconcile"):
        return _bad()
    if command not in _administrative_kinds:
        _administrative_kinds[command] = OperationKind(command, "administrative", _json, _PROOF)
    return _administrative_kinds[command]


@dataclass(frozen=True, repr=False)
class _AdministrativeReference(OperationReference[T], Generic[T]):
    """A distinct typed reference prevents accidental participant recover()."""


class _AdministrativeOperation(Operation[T]):
    def __init__(self, owner: AdministrativeClient, reference: OperationReference[T]):
        # The common Operation only needs helper, decision kinds and from_reply.
        # Administration supplies that private owner interface. Its overridden
        # dispatch below always uses the separate administrative profile.
        super().__init__(cast(Client, owner), reference)

    async def _run(self, action: dict, options: RequestOptions, decode: Callable[[Any], R],
                   key: OperationKey | None = None, mutation: bool = False) -> R:
        return await self._client._helper.request(
            {"type":"administrative_operation", "reference":self.reference._saved, "action":action},
            options, decode, key or self.reference._key, mutation)


@dataclass(frozen=True, repr=False)
class _DecisionKinds:
    operation_cancel: OperationKind[DecisionReceipt] = _KINDS.operation_cancel
    operation_reconcile: OperationKind[DecisionReceipt] = _KINDS.operation_reconcile


@dataclass(frozen=True, repr=False)
class AdministrativeOperations:
    _client: AdministrativeClient

    def restore(self, saved: Json) -> Operation[Json | DecisionReceipt]:
        checked = self._client._validate(saved, reference=True)
        selected = cast(OperationKind[Json | DecisionReceipt], _select(checked.reference["command"], checked.kind))
        return self._client._captured(checked.reference, selected)

    def recover(self, reference: OperationReference[T]) -> Operation[T]:
        if type(reference) is not _AdministrativeReference or reference._proof is not _PROOF:
            raise SdkError("validation", "not-submitted")
        known = _select(reference.command, reference._kind._name)
        if known is not reference._kind:
            raise SdkError("validation", "not-submitted")
        recovered = self._client._captured(reference._saved, reference._kind)
        object.__setattr__(recovered.reference, "_decision_target", reference._decision_target)
        return recovered


@dataclass(frozen=True, init=False, repr=False)
class AdministrativeClient:
    """One explicitly configured administrative owner; no participant facade."""
    _helper: _SdkHelper
    operations: AdministrativeOperations
    operation_kinds: _DecisionKinds

    def __init__(self, config: str | os.PathLike, executable: str | os.PathLike = "connect"):
        object.__setattr__(self, "_helper", _SdkHelper(config, executable, administrative=True))
        object.__setattr__(self, "operations", AdministrativeOperations(self))
        object.__setattr__(self, "operation_kinds", _DecisionKinds())

    def _validate(self, value: Json, *, reference: bool = False) -> _Descriptor:
        maximum = 4096 if reference else 65536
        try:
            captured, raw = _encode(value, maximum)
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise SdkError("validation", "not-submitted") from None
        response = _validate(self._helper._executable, "sdk-validate-provisioning", raw,
                             arguments=("--reference",) if reference else (), maximum_output=128 * 1024)
        try:
            answer = _parse(response, True)
            if (type(answer) is not dict or answer.get("profile") != "connect-sdk-provisioning-validation/1"
                    or type(answer.get("valid")) is not bool):
                return _bad()
            if not answer["valid"]:
                if not _object(answer, ("profile", "valid")):
                    return _bad()
                raise SdkError("validation", "not-submitted")
            if (not _object(answer, ("profile", "valid", "value", "reference", "kind"))
                    or not _object(answer["reference"], ("schema", "operation_id", "command", "scope"))
                    or answer["reference"]["schema"] != "connect-operation-reference/1"
                    or not identifier(answer["reference"]["operation_id"])
                    or type(answer["reference"]["command"]) is not str
                    or re.fullmatch(r"[a-z][a-z0-9.-]{1,127}", answer["reference"]["command"]) is None
                    or not _object(answer["reference"]["scope"], ())
                    or type(answer["kind"]) is not str or answer["kind"] not in ("administrative", "operation_cancel", "operation_reconcile")):
                return _bad()
            normalized, _ = _encode(answer["value"], maximum)
            saved, _ = _encode(answer["reference"], 4096)
            _select(saved["command"], answer["kind"])
            if reference:
                if normalized != saved or captured != saved:
                    return _bad()
            elif (answer["kind"] != "administrative" or type(normalized) is not dict or type(captured) is not dict
                    or normalized.get("operation_id") != saved["operation_id"]
                    or captured.get("operation_id") != saved["operation_id"] or normalized.get("type") != captured.get("type")):
                return _bad()
            return _Descriptor(freeze(normalized), freeze(saved), answer["kind"])
        except SdkError:
            raise
        except (ValueError, TypeError, UnicodeError, RecursionError):
            self._helper._fence("protocol")
            raise SdkError("protocol", "not-submitted") from None

    def validate_provisioning(self, value: Json) -> TrustedProvisioningCommand:
        checked = self._validate(value)
        command = object.__new__(TrustedProvisioningCommand)
        _commands[command] = checked
        return command

    def _captured(self, value: Json, kind: OperationKind[T]) -> Operation[T]:
        try:
            saved, _ = _encode(value, 4096)
            if (not _object(saved, ("schema", "operation_id", "command", "scope"))
                    or saved["schema"] != "connect-operation-reference/1" or not identifier(saved["operation_id"])
                    or saved["command"] != kind.command or not _object(saved["scope"], ())
                    or _select(kind.command, kind._name) is not kind):
                raise ValueError()
            key = OperationKey(saved["operation_id"], kind.command, Scope())
            return _AdministrativeOperation(self, _AdministrativeReference(key, kind, freeze(saved), _PROOF))
        except (ValueError, TypeError, UnicodeError, RecursionError):
            raise SdkError("validation", "not-submitted") from None

    def _from_reply(self, value: Any, kind: OperationKind[T], key: OperationKey) -> Operation[T]:
        try:
            operation = self._captured(value, kind)
            if operation.reference._key != key:
                return _bad()
            return operation
        except SdkError:
            return _bad()

    async def provision(self, command: TrustedProvisioningCommand, options: MutationOptions) -> Operation[Json]:
        if type(command) is not TrustedProvisioningCommand or (checked := _commands.get(command)) is None or type(options) is not MutationOptions:
            raise SdkError("validation", "not-submitted")
        key = OperationKey(options.operation_id, checked.reference["command"], Scope())
        if key.operation_id != checked.reference["operation_id"]:
            raise SdkError("validation", "not-submitted", key)
        if options.extensions:
            raise SdkError("unsupported", "not-submitted", key)
        kind = cast(OperationKind[Json], _select(key.command, "administrative"))
        return await self._helper.request({"type":"administrative_provision", "command":checked.value,
                                           "mutation":{"operation_id":key.operation_id}}, options.request,
                                          lambda value:self._from_reply(value, kind, key), key, True)

    async def close(self, options: RequestOptions) -> None:
        await self._helper.close(options)

    async def __aenter__(self) -> AdministrativeClient:
        return self

    async def __aexit__(self, exc_type, exc, traceback) -> None:
        await self.close(RequestOptions(5000))
