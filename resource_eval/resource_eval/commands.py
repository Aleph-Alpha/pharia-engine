"""Simple command language and interpreter to deploy and execute skills against a running Pharia Kernel.
Measurements per command are stored in the *run* log files, which are meant to be used for evaluation.
Each line is a json string representing resource consumption information, time and memory.
- cmd: which command was executed
- took: how much time it took to execute, wall time
- rss: how much resident memory it uses after this command
- rss_diff: how much resident memory was added or removed after this command
- vms, vms_diff: how much virtual memory...
"""

import datetime
import hashlib
import os
import shutil
import time

import requests

from .log_util import dt2str, logger, run_logger
from .run_pk import PhariaKernel, observe_diff

# these are fixed, we only use sample.wasm
SKILL_NAME_PY = "sample_py"
SKILL_FNAME_PY = SKILL_NAME_PY + ".wasm"
assert os.path.isfile(SKILL_FNAME_PY)

# these are the key constants used
AA_API_TOKEN = "AA_API_TOKEN"
PHARIA_KERNEL_ADDRESS = "PHARIA_KERNEL_ADDRESS"
PHARIA_KERNEL_URL = "PHARIA_KERNEL_URL"
HEADERS = "headers"
COMMAND = "cmd"
MODIFIED = "modified"
HASH = "hash"
FILENAME = "filename"

# we need the skill files already compiled


def _to_num(args):
    "converts args to int or float if possible, else remain str"
    ret = []
    for arg in args:
        try:
            ret.append(int(arg))
            continue
        except ValueError:
            pass
        try:
            ret.append(float(arg))
            continue
        except ValueError:
            pass
        ret.append(arg)
    return ret


def execute_command(pk: PhariaKernel, args: list[str], repeating=False, log=True):
    "execute a command on a Pharia Kernel, may be a repetition once"
    if not args:
        return
    if not repeating:
        try:
            repetition = int(args[0])
            before = pk.observe
            for _ in range(repetition):
                execute_command(pk, args[1:], repeating=True, log=False)
            after = pk.observe
            diff = observe_diff(before, after)
            diff[COMMAND] = " ".join(args)
            # here, we should mostly log
            if log:
                run_logger.info(diff)
                # see also in console the output to the run logger
                logger.info(f"run: {str(diff)}")
            return
        except ValueError:
            pass  # then no repetition
    before = pk.observe
    if args[0] in COMMANDS:
        cmd_key = args[0]
        COMMANDS[cmd_key](*_to_num(args[1:]), **{"log": log})
    else:
        msg = f"cannot execute command: {' '.join(args)}, ignoring"
        logger.error(msg)
        return
    after = pk.observe
    diff = observe_diff(before, after)
    diff[COMMAND] = " ".join(args)
    if log:
        run_logger.info(diff)
        # see also in console the output to the run logger
        logger.info(f"run: {str(diff)}")
    return diff


REQUEST_CONFIG = dict()


def _ensure_request_config():
    if not REQUEST_CONFIG:
        # fill cache config
        token = os.environ.get(AA_API_TOKEN)
        address = os.environ.get(PHARIA_KERNEL_ADDRESS)
        url = PHARIA_KERNEL_ADDRESS
        if not url.startswith("http"):
            # we can only check memory on a locally running kernel
            assert address.startswith("localhost") or address.startswith("127")
            url = "http://" + address
        while url.endswith("/"):
            url = url[:-1]
        headers = {
            "Content-Type": "application/json",
            "Authorization": f"Bearer {token}",
        }
        REQUEST_CONFIG[AA_API_TOKEN] = token
        REQUEST_CONFIG[PHARIA_KERNEL_URL] = url
        REQUEST_CONFIG[HEADERS] = headers
    return REQUEST_CONFIG[PHARIA_KERNEL_URL], REQUEST_CONFIG[HEADERS]


def execute_skill(skill: str, input_val: str, memory=0, wait_time=0, log=True):
    url, headers = _ensure_request_config()
    full_url = f"{url}/execute_skill"
    skill = f"dev/{skill}"
    input = {"topic": input_val, "memory": memory, "wait_time": wait_time}
    payload = {
        "input": input,
        "skill": skill,
    }
    if log:
        logger.info(f"cmd, skill_request: skill={skill}, input={input}")
    try:
        response = requests.post(full_url, json=payload, headers=headers)
        response.raise_for_status()
        return response.json()
    except Exception as err:
        logger.error(err)
        # ignore


def skills(log=True, log_error=True):
    "list all accessible skills removing dev prefix"
    url, headers = _ensure_request_config()
    full_url = f"{url}/skills"
    try:
        response = requests.get(full_url, headers=headers)
        js_list = sorted(
            [skill_name.removeprefix("dev/") for skill_name in response.json()]
        )
        if log:
            logger.info(f"cmd, skills: {js_list}")
        return js_list
    except Exception as err:
        if log_error:
            logger.error(str(err))


def cached_skills(log=True):
    "list all cached skills removing dev prefix"
    url, headers = _ensure_request_config()
    full_url = f"{url}/cached_skills"
    try:
        response = requests.get(full_url, headers=headers)
        js_list = sorted([skill.removeprefix("dev/") for skill in response.json()])
        if log:
            logger.info(f"cmd, cached_skills: {js_list}")
        return js_list
    except Exception as err:
        logger.error(str(err))


def execute_all(input_val: str | None = None, memory=0, wait_time=0, log=True):
    "access all existing skills, without requesting additional time or memory"
    skill_names = skills(False)
    if input_val is None:
        input_val = "John Doe"
    if log:
        logger.info(
            f"cmd, execute_all: {len(skill_names)} skills with {input_val} memory={memory} wait_time={wait_time}"
        )
    for skill_name in skill_names:
        execute_skill(skill_name, input_val, memory, wait_time, log=False)


def add_one_skill_py(log=True):
    "adds an instance of the example python skill and executes it"
    add_one_skill_py.cnt += 1
    skill_name = f"{SKILL_NAME_PY}{add_one_skill_py.cnt}"
    logger.info(f"cmd, add_one_skill_py: adding python skill {skill_name}")
    shutil.copy(SKILL_FNAME_PY, f"skills/{skill_name}.wasm")
    tries, sleep_time = 10, 0.1
    for _ in range(tries):
        time.sleep(sleep_time)  # wait for the kernel to pick it up
        if skill_name in skills(False):
            break
        sleep_time *= 2
    result = execute_skill(skill_name, "Alice")
    logger.info(f"  add_one_skill_py: accessing {skill_name} returned {result}")


add_one_skill_py.cnt = 0

COMMANDS = {
    "add_py": add_one_skill_py,
    "execute_all": execute_all,
    "execute_skill": execute_skill,
    "skills": skills,
    "cached_skills": cached_skills,
}


def execute_cmds_file(pk: PhariaKernel, cmds_file: str):
    "execute a file with benchmarking commands"
    path = os.path.abspath(cmds_file)
    if not os.path.isfile(path):
        raise Exception(f"cmds file {path} does not exist")
    # store information about the running Pharia Kernel binary in the run log
    commands = []
    for line in open(path):
        line = line.strip()
        if not line:
            continue
        if line.startswith("#"):
            continue
        args = line.split()
        commands.append(args)
    cmd_config = {
        FILENAME: cmds_file,
        # focus on the commands, ignore comments and whitespace
        HASH: hashlib.sha1(str(commands).encode()).hexdigest(),
        MODIFIED: dt2str(datetime.datetime.fromtimestamp(os.stat(path).st_ctime)),
    }
    run_logger.info(cmd_config)
    logger.info(f"run: {str(cmd_config)}")
    # ensure the kernel is up, max 17 tries
    for _ in range(17):
        if skills(log=False, log_error=False) is not None:
            # got it, axum is up
            break
        time.sleep(0.1)
    # execute the cmd script line by line, logging is done for each command
    for command in commands:
        execute_command(pk, command)
