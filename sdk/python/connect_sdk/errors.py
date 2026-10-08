"""Stable SDK error codes; transport uncertainty never authorizes a new mutation."""
class ConnectError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code
