"""reading and comparing log files that where created using the same cmds script"""

import json
import os
import string
from typing import TypeAlias

from .log_util import logger, str2dt

JSON: TypeAlias = dict[str, "JSON"] | list["JSON"] | str | int | float | bool | None


# some keys
MODIFIED = "modified"
HASH = "hash"
BIN = "bin"
MCH_ARCH = "arch"
MCH_BRAND = "brand"
MCH_CORES = "cores"
MCH_MEMORY_TOTAL = "memory_total"
MCH_MEMORY_AVAILABLE = "memory_available"
FILENAME = "filename"
TOOK = "took"
RSS = "rss"
RSS_DIFF = "rss_diff"
CMD = "cmd"


class Logfile:
    "representation of a log file"

    def __init__(
        self,
        file_name: str,
        machine_info: JSON,
        bin_info: JSON,
        cmd_info: JSON,
        entries: list[JSON],
    ):
        self.file_name = file_name
        self.machine_info = machine_info
        self.bin_info = bin_info
        self.cmd_info = cmd_info
        self.entries = entries
        assert MCH_ARCH in self.machine_info
        assert BIN in self.bin_info
        assert FILENAME in self.cmd_info
        assert all((TOOK in entry for entry in self.entries))

    def cmd_hash(self):
        return self.cmd_info[HASH]

    def len_entries(self):
        return len(self.entries)

    def __iter__(self):
        return iter(self.entries)


def read_logfile(log_file):
    if not os.path.isfile(log_file):
        logger.error("cannot read {log_file}, skipping")
        return
    with open(log_file) as of:
        machine = json.loads(of.readline())
        binary = json.loads(of.readline())
        cmds = json.loads(of.readline())
        entries = [json.loads(line) for line in of]
        log_file_instance = Logfile(log_file, machine, binary, cmds, entries)
    return log_file_instance


def ensure_same_cmd(log_files: list[Logfile]):
    "are all log files based on the same cmds file"
    hashes = set((log_file.cmd_hash() for log_file in log_files))
    if len(hashes) > 1:
        logger.error(f"Hashes in the log files not unique: {hashes}")
        return False
    lens = set((log_file.len_entries() for log_file in log_files))
    if len(lens) > 1:
        logger.error(f"number of executed commands not identical, aborted run?: {lens}")
        return False
    return True


def delim_thousands(num):
    return "{0:,}".format(int(num))


def get_per_diff(other, base):
    "percentage rounded to full digit percent in 5 chars"
    base, other = int(base), int(other)
    if base == 0:
        if other == 0:
            return " 100"
        return "  inf"
    result = round(other * 100 / base)
    return f"{result:5}"


FMT = "{cmd:40}  {id:2}   {took:>14} {t_per:>8}   {rss_diff:>12} {r_per:>8}"
HEADER = FMT.format(
    cmd="cmd",
    id="id",
    took="took(ms)",
    t_per="vs a(%)",
    rss_diff="rss diff(KB)",
    r_per="vs a(%)",
)

SINGLE_FMT = "{cmd:40}  {id:2}   {took:>14}   {rss_diff:>12}"
SINGLE_HEADER = SINGLE_FMT.format(
    cmd="cmd",
    id="id",
    took="took(ms)",
    rss_diff="rss diff(KB)",
)


def do_report(logs: list[Logfile]):
    "textual for now"
    SINGLE = len(logs) == 1
    comp = f" comparing {len(logs)} log files" if not SINGLE else ""
    print(
        f"Evaluating: cmds={logs[0].cmd_info[FILENAME]} hash={logs[0].cmd_info[HASH]}{comp}"
    )
    ids = string.ascii_lowercase[: len(logs)]
    ending = "_run.log"
    len_ending = len(ending)
    for id, log in zip(ids, logs):
        # assume correct filename formatting
        if log.file_name.endswith(ending):
            when_str = os.path.split(log.file_name)[1][:-len_ending]
            when = str2dt(when_str)
        else:
            when = "unknown"
        arch = log.machine_info[MCH_ARCH]
        brand = log.machine_info[MCH_BRAND][:42]
        cores = log.machine_info[MCH_CORES]
        mem_total = log.machine_info[MCH_MEMORY_TOTAL]
        mem_available = log.machine_info[MCH_MEMORY_AVAILABLE]
        print(f"{id:2} file_name={log.file_name} date={str(when):28}")
        print(f"   brand={brand}")
        print(
            f"   arch={arch:6} cores=cores={cores} mem_total(GB)={mem_total} mem_available(GB)={mem_available}"
        )
        print(f"   binary={log.bin_info[BIN]}")
        print(f"   hash of binary={log.bin_info[HASH]}")
    print("=" * (len(HEADER) if not SINGLE else len(SINGLE_HEADER)))
    print(HEADER if not SINGLE else SINGLE_HEADER)
    print("-" * (len(HEADER) if not SINGLE else len(SINGLE_HEADER)))
    cum_took = {id: 0 for id in ids}
    rss_max = {id: 0 for id in ids}
    for all_per_entry in zip(*logs):
        first = True
        for idx, entry in enumerate(all_per_entry):
            id = ids[idx]
            if first:
                first_took, first_rss_diff = entry[TOOK], entry[RSS_DIFF]
            took, rss, rss_diff = (
                int(entry[TOOK]),
                int(entry[RSS]),
                int(entry[RSS_DIFF]),
            )
            cum_took[id] += took
            rss_max[id] = max(rss_max[id], rss)
            if SINGLE:
                print(
                    SINGLE_FMT.format(
                        cmd=entry[CMD] if first else "",
                        id=id,
                        took=delim_thousands(took),
                        rss_diff=delim_thousands(rss_diff),
                    )
                )
            else:
                print(
                    FMT.format(
                        cmd=entry[CMD] if first else "",
                        id=id,
                        took=delim_thousands(took),
                        t_per=get_per_diff(took, first_took) if not first else "",
                        rss_diff=delim_thousands(rss_diff),
                        r_per=get_per_diff(rss_diff, first_rss_diff)
                        if not first
                        else "",
                    )
                )
            first = False
        # print(all_per_entry)
    print("-" * (len(HEADER) if not SINGLE else len(SINGLE_HEADER)))
    for id in ids:
        what = "Total time (ms)" if id == "a" else ""
        t_per = get_per_diff(cum_took[id], cum_took["a"]) if id != "a" else ""
        print(f"{what:40}  {id:2}   {delim_thousands(cum_took[id]):>14} {t_per:>8}")
    space = " " * (24 if not SINGLE else 15)
    for id in ids:
        what = "Maximum rss memory (KB)" if id == "a" else ""
        t_per = get_per_diff(rss_max[id], rss_max["a"]) if id != "a" else ""
        print(
            f"{what:40}  {id:2} {space}  {delim_thousands(rss_max[id]):>14} {t_per:>8}"
        )


def do_eval(log_files):
    logs = [read_logfile(f) for f in log_files]
    logs = [one_log for one_log in logs if one_log]
    if not ensure_same_cmd(logs):
        logger.error("log files created with different commands, ending")
        return
    do_report(logs)


def test_eval():
    do_eval(["../logs/241022_204217.730_run.log", "../logs/241022_204217.737_run.log"])


if __name__ == "__main__":
    test_eval()
