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
    "converts a datetime instance to a string that can be parsed, without whitespace"
    if timestamp is None:
        timestamp = datetime.datetime.now()
    ms = f"{timestamp.microsecond // 1_000:03d}"
    return timestamp.strftime("%y%m%d_%H%M%S") + f".{ms}"


def str2dt(s: str) -> datetime.datetime:
    left, ms = s.split(".", 1)
    dt = datetime.datetime.strptime(left, "%y%m%d_%H%M%S")
    dt = dt.replace(microsecond=int(ms) * 1000)
    return dt


# for console logging of the driving application, we always need that
logger = _setup_console_logger("main")


class NoneLogger:
    def __init__(self):
        self.logger = None

    def info(self, *args, **kwargs):
        if self.logger:
            self.logger.info(*args, **kwargs)

    def error(self, *args, **kwargs):
        if self.logger:
            self.logger.error(*args, **kwargs)


# the log file for a run, all lines are json; app specific timestamp in all
# lines where Pharia Kernel operations are executed. To be used for evaluation.
# Only needed when running
run_logger = NoneLogger()

# Pass through of the log lines from Pharia Kernel and regular resource updates, for tracing
pk_logger = NoneLogger()


def init_run_loggers():
    run_logger.logger = _setup_json_logger("run", "run.log")
    pk_logger.logger = _setup_file_logger("pk", "pk.log")


def _test_log():
    import time

    logger.info("hello")
    init_run_loggers()
    dic = {"timestamp": dt2str(), "msg": "hello", "a": 17, "b": 42}
    run_logger.info(dic)
    pk_logger.info(dic)
    time.sleep(0.1)
    dic["timestamp"] = dt2str()
    run_logger.info(dic)
    pk_logger.info(dic)
    logger.info("hello again")
    time.sleep(0.1)
    logger.info("end")


if __name__ == "__main__":
    _test_log()
