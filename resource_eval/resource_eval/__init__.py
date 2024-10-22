import argparse

from dotenv import load_dotenv

from .commands import execute_cmds_file
from .log_util import dt2str, logger, run_logger
from .run_pk import PhariaKernel, diff_str_dt, get_machine


def do_run(cmds_file, mem):
    start = dt2str()
    logger.info("start of run")
    machine = get_machine()
    logger.info(f"run: {str(machine)}")
    run_logger.info(machine)
    pk = PhariaKernel(mem)
    pk.start()
    logger.info("Pharia Kernel started")
    logger.info(f"run: {str(pk.config)}")
    run_logger.info(pk.config)
    logger.info(f"executing benchmark commands file {cmds_file}")
    execute_cmds_file(pk, cmds_file)
    logger.info("shutting down Pharia Kernel")
    pk.quit()
    end = dt2str()
    logger.info(f"end of run, took {diff_str_dt(start, end)} ms total")


def do_eval(log_files):
    print("log_files:", log_files)


def main():
    load_dotenv()
    parser = argparse.ArgumentParser(
        description="""Evaluate Pharia Kernel Resource Consumption

        Executes simple commands in a *.cmds file, see bench.cmds for an example.
        Each log file starts with yymmdd-HHMMSS.ms_ which represents the start time
        Always a *run.log and a *pk.log is created, in addition information is given
        on the terminal. In the logs on the terminal logs with 
        * "cmd" are from running a command in a commands file
        * "run" are what is found in the *run.log file 
        * "resource" are optional regular updates in between commands, activated with -m
        all other logs are purely informational.
        You may check the *pk.log file if you want to see the Pharia Kernel log.
        For evaluation the information in the *run.log file is
        """
    )
    subparsers = parser.add_subparsers(dest="command", help="Available commands")
    run_parser = subparsers.add_parser(
        "run", help="Run a cmds file, executing a resource consumption script"
    )
    run_parser.add_argument(
        "--mem",
        "-m",
        action="store_true",
        help="log memory consumption on the console regularly",
    )
    run_parser.add_argument(
        "cmds_file",
        nargs="?",
        default="bench.cmds",
        help="File, that contains the commands to simulate skill deployments and accesses",
    )
    evaluate_parser = subparsers.add_parser(
        "eval", help="compare one ore more resource consumption runs"
    )
    evaluate_parser.add_argument(
        "log_files",
        nargs="+",
        help="log files that contain the logs of a cmd run, do not mix logs of different cmd files",
    )
    args = parser.parse_args()
    if args.command == "run":
        do_run(args.cmds_file, args.mem)
    elif args.command == "eval":
        do_eval(args.log_files)
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
