"""Run Pharia Kernel for local development and observe its resource consumption"""

import datetime
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import threading
import time

import dotenv
import psutil

from .log_util import dt2str, logger, pk_logger, run_logger, str2dt

# environment keys
PHARIA_KERNEL_BIN = "PHARIA_KERNEL_BIN"
PHARIA_KERNEL_ADDRESS = "PHARIA_KERNEL_ADDRESS"

# other keys
TIMESTAMP = "timestamp"
TOOK = "took"
BIN = "bin"
HASH = "hash"
MODIFIED = "modified"

# constants
WAIT_BETWEEN_MEMORY_CHECKS = 0.5
SHUTDOWN_TIMEOUT = 2.5

# where to look for the Pharia Kernel binary
PK_LOCATIONS = [
    "../target/debug/pharia-kernel",
    "../target/release/pharia-kernel",
    "pharia-kernel",
]

# to get rid of the ansi color stuff from the logs
ANSI_ESCAPE_8BIT = re.compile(
    rb"(?:\x1B[@-Z\\-_]|[\x80-\x9A\x9C-\x9F]|(?:\x1B\[|\x9B)[0-?]*[ -/]*[@-~])"
)


def de_escape(line: str):
    "removes the ansi escape commands from a string"
    return ANSI_ESCAPE_8BIT.sub(b"", line).decode()


def get_mem(proc):
    "returns the memory resident size of proc in KBytes"
    if proc is None:
        raise Exception("proc must not be None")
    try:
        mem_info = proc.memory_info()
        # these are the only cross platform ones
        rss = mem_info.rss // 1024
        vms = mem_info.vms // 1024
        return rss, vms
    except psutil.NoSuchProcess:
        return 0, 0


def observe_diff(start: dict, end: dict):
    "what are the values and differences in a dictionary of logged values"
    dic = dict()
    for key in start.keys():
        if key == TIMESTAMP:
            sdt, edt = str2dt(start[key]), str2dt(end[key])
            delta = edt - sdt
            # in milliseconds
            dic[TOOK] = int(delta / datetime.timedelta(milliseconds=1))
        elif key not in end:
            continue
        elif type(start[key]) in (int, float):
            dic[key + "_diff"] = end[key] - start[key]
            dic[key] = end[key]
        else:
            # nothing to compute
            pass
    return dic


class PhariaKernel:
    "instantiate and run Pharia Kernel"

    def __init__(self, mem_trace=False):
        self.mem_trace = mem_trace
        # get the binary
        self.pk_bin = os.environ.get(PHARIA_KERNEL_BIN)
        if not self.pk_bin:
            for to_check in PK_LOCATIONS:
                if to_check[0] in (".", "/"):  # path
                    if os.path.isfile(to_check) and os.access(to_check, os.X_OK):
                        self.pk_bin = os.path.abspath(to_check)
                        break
                else:
                    self.pk_bin = shutil.which("pharia-kernel")
        if not self.pk_bin:
            raise Exception("cannot find a Pharia Kernel binary %s, abort", self.pk_bin)
        self.pk_bin = os.path.realpath(self.pk_bin)
        self.pk_bin_hash = hashlib.sha1(open(self.pk_bin, "rb").read()).hexdigest()
        self.pk_bin_changed = datetime.datetime.fromtimestamp(
            os.stat(self.pk_bin).st_ctime
        )
        # where to send requests later
        self.pk_addr = os.environ.get(PHARIA_KERNEL_ADDRESS)
        # we need a skills directory and must not have an operator-config nor namespace.toml
        if os.path.isfile("operator-config.toml"):
            raise Exception("there must not be an operator-config.toml")
        if os.path.isfile("namespace.toml"):
            raise Exception("there must not be a namespace.toml")
        if os.path.isfile("skills"):
            raise Exception("there must not be a file called skills")
        if os.path.isdir("skills"):
            shutil.rmtree("skills")
        # we make the skill folder fresh
        pathlib.Path("skills").mkdir(parents=False, exist_ok=False)

        # initially nothing is running
        self.proc = None

    @property
    def config(self):
        "information about the binary running"
        return {
            BIN: self.pk_bin,
            HASH: self.pk_bin_hash,
            MODIFIED: dt2str(self.pk_bin_changed),
            # do wee need the address? always 127.0.0.1?
            # PHARIA_KERNEL_ADDRESS: self.pk_addr,
        }

    @property
    def observe(self, process=None):
        "observe the current resource consumption"
        now = datetime.datetime.now()
        rss, vms = get_mem(process or self.process)
        dic = {"timestamp": dt2str(now), "type": "resources", "rss": rss, "vms": vms}
        return dic

    def _capture_logs(self):
        "capture the logs in a separate thread, all access is threadsafe"
        while not self.end_pk.is_set():
            line = self.proc.stdout.readline()
            line = de_escape(line)
            # get rid of first line without kernel logging set up
            if line.startswith("Info"):
                continue
            line = line.strip()
            if not line:
                break
            # no need for that long timestamp
            _now, rest = line.split(maxsplit=1)
            pk_logger.info(rest)

    def _capture_resources(self, process):
        "capture memory consumption in a separate thread, log every change"
        last = {"rss": 0, "vms": 0}
        while not self.end_pk.is_set():
            rss, vms = get_mem(process)
            dic = {"rss": rss, "vms": vms}
            if last["rss"] != rss or last["vms"] != vms:
                pk_logger.info(json.dumps(dic))
                if self.mem_trace:
                    logger.info(f"resource: {json.dumps(dic)}")
            last = dic
            self.end_pk.wait(WAIT_BETWEEN_MEMORY_CHECKS)

    def start(self):
        "run a pharia kernel in a separate process, capture the output, observe memory consumption"
        self.proc = subprocess.Popen(
            [self.pk_bin], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, shell=False
        )
        self.end_pk = threading.Event()
        self.process = psutil.Process(self.proc.pid)

        self.log_thread = threading.Thread(target=self._capture_logs)
        self.log_thread.start()

        self.resource_thread = threading.Thread(
            target=self._capture_resources, args=(self.process,)
        )
        self.resource_thread.start()

    def quit(self):
        "ends a running pharia kernel process, may take time if memory has been swapped"
        if self.proc is None:
            # not running
            return
        # note, that terminating can take a while if memory needs to be paged in
        before = self.observe
        self.proc.terminate()
        while True:
            try:
                self.proc.wait(timeout=SHUTDOWN_TIMEOUT)
                break
            except subprocess.TimeoutExpired:
                logger.warning("waiting for Pharia Kernel to terminate")
        # then we can end all logging
        after = self.observe
        diff = observe_diff(before, after)
        diff["cmd"] = "stop"
        run_logger.info(diff)
        logger.info(f"run: {str(diff)}")
        self.end_pk.set()
        self.log_thread.join()
        self.resource_thread.join()
        self.proc = None
        self.process = None

    def __str__(self):
        s = ", ".join(f"{key}={val}" for key, val in self.config.items())
        return f"PhariaKernel(pid={self.proc.pid if self.proc else "None"}, {s})"

    __repr__ = __str__


def test_run():
    dotenv.load_dotenv()
    pk = PhariaKernel()
    print(pk.config)
    pk.start()
    time.sleep(2.0)
    print("\n".join(pk.pk_mem))
    time.sleep(2.0)
    pk.quit()
    pk.show_logs()


if __name__ == "__main__":
    test_run()
