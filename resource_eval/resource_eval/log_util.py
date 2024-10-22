import datetime
import json
import logging


class _CustomJsonFormatter(logging.Formatter):
    # https://stackoverflow.com/questions/50144628/python-logging-into-file-as-a-dictionary-or-json
    def format(self, record: logging.LogRecord) -> str:
        "just format all contents of the msg"
        super(_CustomJsonFormatter, self).format(record)
        output = {k: str(v) for k, v in record.msg.items()}
        return json.dumps(output)


def _setup_json_logger(name, log_file):
    log_file = dt2str(datetime.datetime.now()) + "_" + log_file
    json_handler = logging.FileHandler(log_file)
    json_formatter = _CustomJsonFormatter()
    json_handler.setFormatter(json_formatter)
    logger = logging.getLogger(name)
    logger.setLevel(logging.INFO)
    logger.addHandler(json_handler)
    return logger


def _setup_file_logger(name, log_file):
    log_file = dt2str(datetime.datetime.now()) + "_" + log_file
    handler = logging.FileHandler(log_file)
    formatter = logging.Formatter(
        "%(asctime)s.%(msecs)03d %(message)s", "%y%m%d_%H%M%S"
    )
    handler.setFormatter(formatter)
    logger = logging.getLogger(name)
    logger.setLevel(logging.INFO)
    logger.addHandler(handler)
    return logger


def _setup_console_logger(name):
    handler = logging.StreamHandler()
    formatter = logging.Formatter(
        "%(asctime)s.%(msecs)03d %(message)s", "%y%m%d_%H%M%S"
    )
    handler.setFormatter(formatter)
    logger = logging.getLogger(name)
    logger.setLevel(logging.INFO)
    logger.addHandler(handler)
    return logger


def dt2str(timestamp: datetime.datetime = None) -> str:
    "converts a datetime instance to a parseable string without whitespace"
    if timestamp is None:
        timestamp = datetime.datetime.now()
    ms = f"{timestamp.microsecond // 1_000:03d}"
    return timestamp.strftime("%y%m%d_%H%M%S") + f".{ms}"


def str2dt(s: str) -> datetime.datetime:
    left, ms = s.split(".", 1)
    dt = datetime.datetime.strptime(left, "%y%m%d_%H%M%S")
    dt = dt.replace(microsecond=int(ms) * 1000)
    return dt


# for console logging  of the driving application
logger = _setup_console_logger("main")

# the log file for a run, all lines are json; app specific timestamp in all entries
# that one is used for evaluation
run_logger = _setup_json_logger("run", "run.log")

# in case someone is interested, we also just pass through the entries from Pharia Kernel
pk_logger = _setup_file_logger("pk", "pk.log")


def _test_log():
    import time

    logger.info("hallo")
    time.sleep(0.1)
    logger.info("und so")

    run_logger.info({"timestamp": dt2str(), "msg": "hallo", "a": 17})
    time.sleep(0.1)
    run_logger.info({"timestamp": dt2str(), "msg": "hallo"})


if __name__ == "__main__":
    _test_log()
