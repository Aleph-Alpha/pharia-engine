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

You find a simple textual comparison of the above log files enclosed.

```text
Evaluating: cmds=bench.cmds hash=3c6d21c18c26d4130b6f57d617d2131cedd58d1d comparing 2 log files
a  fname=logs/241022_204217.730_run.log date=2024-10-22 20:42:17.730000   brand=Apple M3 Pro
   arch=ARM_8  cores=cores=12 mem_total(GB)=36 mem_available(GB)=13
   binary=/Users/peter.barth/dev/pharia-kernel/target/debug/pharia-kernel hash=753c4e8aaf004e75367a3cb0de8c22db44d135aa
b  fname=logs/241022_204217.737_run.log date=2024-10-22 20:42:17.737000   brand=AMD Ryzen 7 PRO 5750G with Radeon Graphics
   arch=X86_64 cores=cores=16 mem_total(GB)=15 mem_available(GB)=11
   binary=/home/peter/dev/pharia-kernel/target/debug/pharia-kernel hash=8f652eac0741839d18d96d2f7a9cff291fdc4e7c
==============================================================================================
cmd                                       id         took(ms)  vs a(%)   rss diff(KB)  vs a(%)
----------------------------------------------------------------------------------------------
add_py                                    a             3,171                 834,992         
                                          b             3,062       97        885,748      106
4 add_py                                  a            16,086                 611,616         
                                          b            16,145      100        598,600       98
skills                                    a                 2                       0         
                                          b                 1       50              0      100
execute_skill sample_py1 Alice 100 20     a             2,892                  90,544         
                                          b                26        1            524        1
execute_all Bob 20 10                     a                76                      96         
                                          b                85      112            224      233
10 execute_all Cecilia 0 0                a               140                       0         
                                          b               196      140            432      inf
skills                                    a                 1                       0         
                                          b                 1      100              0      100
cached_skills                             a                 0                       0         
                                          b                 1      inf              0      100
stop                                      a             6,067              -1,537,472         
                                          b               114        2     -1,488,328       97
----------------------------------------------------------------------------------------------
Total time (ms)                           a            28,435         
                                          b            19,631       69
Maximum rss memory (KB)                   a                                 1,537,472         
                                          b                                 1,488,328       97
```

The biggest problem, which is reproducible and happens always, is stopping Pharia Kernel
at the end of a run. For `bench.cmds` both machines used about 1.5 GB of resident memory.
Stopping that instance (sending SIGTERM, which is what happens if you press Ctrl-C) took
more than 6 seconds on the Mac but less than 120 ms on the x86_64 machine. On a Mac, it
helps to delete the skills from cache and remove them before shutting down.
An assumption is, that the behavior depends on the operating system, not the instruction set architecture.
As customers will most likely use Linux machines and in addition, customers will most likely
deploy on x86_64 machines, it should be fine.

We run Pharia Kernel also on an older x86_64 machine on a KVM instance running `Debian 12` providing
16 cores and 128 GB of memory. No other load was running on that instance, we may safely assume,
that no memory paging took place.

```text
241022_211735.835_run.log      x86_64 EPYC 7301
```

```text
Evaluating: cmds=stress.cmds hash=fe671f5712f7fca8f7e5ac64be07a72186df2be6
a  fname=logs/241022_211735.835_run.log date=2024-10-22 21:17:35.835000   brand=AMD EPYC 7301 16-Core Processor
   arch=X86_64 cores=cores=16 mem_total(GB)=126 mem_available(GB)=124
   binary=/home/peter/pharia-kernel/target/debug/pharia-kernel hash=b6c7be763e517ee2df15ed3f7b40b7d5d3da5e4c
==============================================================================================
cmd                                       id         took(ms)  vs a(%)   rss diff(KB)  vs a(%)
----------------------------------------------------------------------------------------------
128 add_py                                a           822,576              11,455,548         
skills                                    a                 3                       0         
execute_skill sample_py1 Alice 100 20     a                38                       4         
10 execute_all Bob 0 0                    a            18,319                 157,096         
10 execute_all Cecilia 0 0                a             6,400                      56         
10 execute_all Dominic 1000 0             a             7,759                      44         
128 add_py                                a           818,621               9,929,496         
10 execute_all Bob 0 0                    a            50,908                 586,344         
10 execute_all Cecilia 0 0                a            15,117                      76         
10 execute_all Dominic 1000 0             a            17,275                      24         
cached_skills                             a                 3                       0         
stop                                      a             1,220             -22,131,504         
----------------------------------------------------------------------------------------------
Total time (ms)                           a         1,758,239         
Maximum rss memory (KB)                   a                                22,131,504         
```

Adding the first 128 Python skills took 822 seconds and consumed 11.4 GB of resident memory.
Access the first skill after loading another 127 skills was basically instant (38 ms) and consumed
almost no additional memory (4 KB).
Accessing 10 times all 128 skills sequentially took 18.3 seconds and consumed 157 MB additional resident memory,
which appears to be fine with only 15 ms per skill invocation (without hitting external services such as inference,
we measure just the overhead). This provides excellent quick startup time when accessing a loaded skill.
Adding the next 128 Python skills took again about 818 seconds and consumed another 10 GB of resident memory.
Basically, linear scaling as one wants it.
Executing all 256 skills 10 times took 50 seconds and added some 586 MB of main memory consumption.
Executing all 256 skill again 10 times (all is hot) took only 15 seconds and no significant memory increase.
Some memory rearrangement seems to take place, but nothing out of the ordinary.
Executing all 256 skill yet again 10 times for the third time, but this time each one consumes 1 MB of memory took 17 seconds
and yet again no significant memory increase. This is at it should be as skill invocation is one shot, there is no state.
Thus, it may be assumed, that on sequential access resources are freed quickly.
Furthermore, shutting down an instance hosting 256 skills and consuming 22 GB of resident memory took only 1.2 seconds.

All together, loading and running 256 skills used about 22 GB main memory, which is less than 100 MB per skill.
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

```text
Evaluating: cmds=cmds/test_p_execute_all.cmds hash=c43cb5d37cabddd93e11c576235223e0bd695992
a  fname=logs/241023_155316.286_run.log date=2024-10-23 15:53:16.286000   brand=Apple M3 Pro
   arch=ARM_8  cores=cores=12 mem_total(GB)=36 mem_available(GB)=13
   binary=/Users/peter.barth/dev/pharia-kernel/target/release/pharia-kernel hash=9290db81ab9e6c038ca3d0f1ad825aded8003317
============================================================================
cmd                                       id         took(ms)   rss diff(KB)
----------------------------------------------------------------------------
p_add_py 10                               a            23,737      1,946,880
execute_all JohnDoe 0 0                   a                23              0
15 execute_all JohnDoe 0 0                a               320              0
p_execute_all MaxHeadroom 0 0 5           a                79            128
p_execute_all MaxHeadroom 0 0 10          a               152              0
p_execute_all MaxHeadroom 0 0 15          a               227             80
15 execute_all JohnDoe 1000 100           a            16,901             48
p_execute_all MaxHeadroom 1000 100 5      a               210              0
p_execute_all MaxHeadroom 1000 100 10     a               276              0
p_execute_all MaxHeadroom 1000 100 15     a               353         11,760
10 drop_cached_skill                      a            27,796       -744,800
10 delete_skill                           a                12              0
stop                                      a                19     -1,225,808
----------------------------------------------------------------------------
Total time (ms)                           a            70,105         
Maximum rss memory (KB)                   a                        1,970,608    
```

We focus on execution time when accessing 10 different skills several times. The baseline is sequential
access without additional resource consumption. Accessing 10 skills takes 23 ms, doing that 15 times takes 320 ms.
We can also do all 150 accesses in parallel, which takes 227 ms. Not a huge difference and none expected.
More interesting is an individual skill invocation, which uses 1 MB of memory and takes 100 ms to complete.
It is no surprise, that sequentially accessing 10 skills 15 times takes around 16.9 seconds. What is nice, that
doing this 150 accesses in parallel takes only 353 ms and we measure an increase in resident memory consumption of
only 11.7 MB. Ideal scaling would mean 100 ms, no scaling at all would mean at least 15 seconds. 353 ms appears
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
