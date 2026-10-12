"""Wire primitives shared by control and task validators."""
def fields(value, strings=(), nullable=()):
    return isinstance(value, dict) and all(isinstance(value.get(k), str) for k in strings) and all(k in value and (value[k] is None or isinstance(value[k], str)) for k in nullable)


def array(value, test):
    return isinstance(value, list) and all(test(item) for item in value)


def integer(value):
    return type(value) is int and value >= 0


def shape(value, strings=(), nullable=(), extra=()):
    return fields(value, strings, nullable) and set(value).issubset((*strings, *nullable, *extra))
