# Evaluating Pharia Kernel

## Findings Memory Consumption

An almost empty Python Skill, but at least with a native `pydantic` dependency consumes as
compiled Wasm file around 50 MB. Successively adding such skills to Pharia Kernel increases
memory consumption first by about 800 MB, second by about 300 MB and each following copy
by about 150 MB.

The file `bench.cmds` adds first one, then another 4 skills to Pharia Kernel.
Each skill is instantly accessed to ensure resource consumption. Executing a skill
is very fast as soon as the skill is ready. On the Mac there seems to be some very
aggressive swap out (?) of memory happening, as from time to time resource consumption
and wall time changes a lot. For example, the first time executing the skill `sample_py1``
after adding another 4 has on the Mac already caused some 2.8 seconds wall time to get
some 90 MB of memory in or our, which is not the case on the x86_64 machine where it took
less than 30 ms and only 0.5 MB resident memory change. The run to run variance on the Mac
is massive, it may also show the same behavior as on the x86_64 machine running Linux.
For reference the enclosed log files.

```text
241022_204217.730_run.log      ARM    M3 Pro
241022_204217.737_run.log      x86_64 5750G
```

The biggest problem, which is reproducible and happens always, is stopping Pharia Kernel
at the end of a run. For `bench.cmds` both machines used about 1.5 GB of resident memory.
Stopping that instance (sending SIGTERM, which is what happens if you press Ctrl-C) took
more than 6 seconds on the Mac but only about 120 ms on the x86_64 machine. On a Mac, it
helps to delete the skills from cache and remove them before shutting down.
An assumption is, that the behavior depends on the operating system, not the instruction set architecture.
As customers will most likely use Linux machines and in addition, customers will most likely
deploy on x86_64 machines, so it should be fine.

We run Pharia Kernel also on an older x86_64 machine on a KVM instance running `Debian 12` providing
16 cores and 128 GB of memory. No other load was running on that instance, we may safely assume,
that no memory paging took place.

```text
241022_211735.835_run.log      x86_64 EPYC 7301
```

Adding the first 128 Python skills took 822 seconds and consumed 11.4 GB of resident memory.
Access the first skill after loading another 127 skills was basically instant and consumed no additional memory.
Accessing 10 times all 128 skills sequentially took 18 seconds and consumed 157 MB additional resident memory,
which appears to be fine with only 15 ms per skill invocation (without hitting external services such as inference,
we measure just the overhead). This provides excellent quick startup time when accessing a loaded skill.
Adding the next 128 Python skills took again about 818 seconds and consumed another 10 GB of resident memory.
Basically, linear scaling as one wants it.
Executing all 256 skills 10 times took 50 seconds and added some 500MB of main memory consumption.
Executing all 256 skill again 10 times for the second time took only 15 seconds and no significant memory increase.
Some memory rearrangement seems to take place, but nothing out of the ordinary.
Executing all 256 skill yet again 10 times for the third time, but this time each one consuming 1MB of memory took 17 seconds
and yet again no significant memory increase. I may be assumed, that on sequential access resources are freed quickly.
Furthermore, shutting down an instance hosting 256 skills and consuming 22 GB of resident memory took only 1.2 seconds.

All together, 256 skills loaded and executed used about 22 GB main memory, which is less than 100 MB per skill.
Even for more resource intensive skills (more libraries, etc.), we may assume that 150-200 MB per skill is sufficient.
This is also based on the assumption, that skill execution itself is not very memory intensive, as the actual
services such as document index and inference are hosted elsewhere.
As a conservative sizing recommendation a 64 GB dedicated x86_64 server instance should be able to host up to 500
Python skills.

Todo: rust skills

## Findings Execution Performance

For execution performance, we try to execute skills in parallel. In the following example log

```text
241023_155316.286_run.log 
```

we focus on execution time when accessing 10 different skills several times. The baseline is sequential
access without additional resource consumption. Accessing 10 skills takes 23 ms, doing that 15 times takes 320 ms.
We can also do all this accesses 150 accesses in parallel, which takes 227 ms. No difference and none expected.
More interesting is an individual skill invocation, which uses 1 MByte of memory and takes 100ms to complete.
It is no surprise, that sequentially accessing 10 skills 15 times takes around 16.9 seconds. What is nice, that
doing this 150 access in parallel takes only 353 ms and we measure an increase in resident memory consumption of
about 11 MB. Ideal scaling would mean 100 ms, no scaling at all would mean at least 15 seconds. 353 ms appears
to be very acceptable.

In the cmds file

```text
saturate.cmds
```

we increase the parallel load steadily. For that, we use 10 skills but do not use additional memory during a
request, but 3 seconds execution time.
Thus, we ensure that most requests run in parallel and have not finished yet.
Pharia Kernel (Oct/24) can handle 200 requests but ceases to answer to requests while trying to answer 300 requests.
This experiment was also executed on a Mac.

```text
logs/241023_161626.638_run.log_failed 
```

It is likely, that tuning dedicated machines and the Web-Server configuration can provide for a more parallel requests.
