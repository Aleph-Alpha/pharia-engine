# Evaluating Memory Consumption and Throughput in Pharia Kernel

For resource intensive skills written in Python a reasonably sized server (64 GB main memory)
on a typical OS/architecture combination (Linux/x86_64) running Pharia Kernel should be able to
host up to 500 skills loaded at the same time.
Most skills are not CPU bound but wait for external resources to become available. Pharia Kernel
should be able to handle up to 1,000 parallel requests (over a variety of these 500 skills).

## Findings Memory Consumption

An almost empty Python skill, but at least with a native `pydantic` dependency consumes as
compiled Wasm file around 50 MB. Successively adding such skills to Pharia Kernel increases
memory consumption first by about 800 MB, second by about 300 MB and each following copy
by about 100 MB.

The file `bench.cmds` adds first one, then another four skills to Pharia Kernel.
Each skill is instantly accessed to ensure resource consumption. Executing a skill
is very fast as soon as the skill is ready. On the Mac there seems to be some very
aggressive swap out (?) of memory happening, as from time to time resource consumption
and wall time changes a lot. For example, the first time executing the skill `sample_py1``
after adding another four, takes on the Mac about 2.8 seconds wall time to get
about 90 MB of memory in or out. This is not the case on a x86_64 machine, where it took
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
a  file_name=logs/241022_204217.730_run.log date=2024-10-22 20:42:17.730000
   brand=Apple M3 Pro
   arch=ARM_8  cores=cores=12 mem_total(GB)=36 mem_available(GB)=13
   binary=/Users/peter.barth/dev/pharia-kernel/target/debug/pharia-kernel
   hash of binary=753c4e8aaf004e75367a3cb0de8c22db44d135aa
b  file_name=logs/241022_204217.737_run.log date=2024-10-22 20:42:17.737000
   brand=AMD Ryzen 7 PRO 5750G with Radeon Graphics
   arch=X86_64 cores=cores=16 mem_total(GB)=15 mem_available(GB)=11
   binary=/home/peter/dev/pharia-kernel/target/debug/pharia-kernel
   hash of binary=8f652eac0741839d18d96d2f7a9cff291fdc4e7c
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
Stopping that instance (sending SIGTERM, similar to Ctrl-C which actually sends SIGINT) took
more than 6 seconds on the Mac but less than 120 ms on the x86_64 machine. On a Mac, it
helps to delete the skills from cache and remove them before shutting down.
An assumption is, that the behavior depends on the operating system, not the instruction
set architecture.
As customers will most likely use Linux machines and in addition, customers will most likely
deploy on x86_64 machines, it should be fine.

We run Pharia Kernel also on an older x86_64 machine on a KVM instance running `Debian 12`
providing 16 cores and 128 GB of memory. No other load was running on that instance, we may
safely assume, that no memory paging took place.

```text
241022_211735.835_run.log      x86_64 EPYC 7301
```

```text
Evaluating: cmds=stress.cmds hash=fe671f5712f7fca8f7e5ac64be07a72186df2be6
a  file_name=logs/241022_211735.835_run.log date=2024-10-22 21:17:35.835000
   brand=AMD EPYC 7301 16-Core Processor
   arch=X86_64 cores=cores=16 mem_total(GB)=126 mem_available(GB)=124
   binary=/home/peter/pharia-kernel/target/debug/pharia-kernel
   hash of binary=b6c7be763e517ee2df15ed3f7b40b7d5d3da5e4c
============================================================================
cmd                                       id         took(ms)   rss diff(KB)
----------------------------------------------------------------------------
128 add_py                                a           822,576     11,455,548
skills                                    a                 3              0
execute_skill sample_py1 Alice 100 20     a                38              4
10 execute_all Bob 0 0                    a            18,319        157,096
10 execute_all Cecilia 0 0                a             6,400             56
10 execute_all Dominic 1000 0             a             7,759             44
128 add_py                                a           818,621      9,929,496
10 execute_all Bob 0 0                    a            50,908        586,344
10 execute_all Cecilia 0 0                a            15,117             76
10 execute_all Dominic 1000 0             a            17,275             24
cached_skills                             a                 3              0
stop                                      a             1,220    -22,131,504
----------------------------------------------------------------------------
Total time (ms)                           a         1,758,239    
Maximum rss memory (KB)                   a                       22,131,504
```

Adding the first 128 Python skills took 822 seconds and consumed 11.4 GB of resident memory.
Access the first skill after loading another 127 skills was basically instant (38 ms) and consumed
almost no additional memory (4 KB).
Accessing 10 times all 128 skills sequentially took 18.3 seconds and consumed 157 MB additional
resident memory, which appears to be fine with only 15 ms per skill invocation (without hitting
external services such as inference, we measure just the overhead). This provides excellent
quick startup time when accessing a loaded skill.
Adding the next 128 Python skills took again about 818 seconds and consumed another 10 GB
of resident memory.
Basically, linear scaling as one wants it.
Executing all 256 skills 10 times took 50 seconds and added some 586 MB of main memory consumption.
Executing all 256 skill again 10 times (all is hot) took only 15 seconds and no significant
memory increase.
Some memory rearrangement seems to take place, but nothing out of the ordinary.
Executing all 256 skill yet again 10 times for the third time, but this time each one
consumes 1 MB of memory took 17 seconds and yet again no significant memory increase.
This is at it should be as skill invocation is one shot, there is no state.
Thus, it may be assumed, that on sequential access resources are freed quickly.
Furthermore, shutting down an instance hosting 256 skills and consuming 22 GB
of resident memory took only 1.2 seconds.

All together, loading and running 256 skills used about 22 GB main memory, which is
less than 100 MB per skill.
Even for more resource intensive skills (more libraries, etc.), we may assume that
150-200 MB per skill is sufficient.
This is also based on the assumption, that skill execution itself is not very memory intensive,
as the actual services such as document index and inference are hosted elsewhere.
As a conservative sizing recommendation a 64 GB dedicated x86_64 server instance should be able
to host up to 500 Python skills.

A Rust skill is also available to test against. However, the memory footprint is much smaller,
the cpu execution requirements are way lower.
Thus we focus for memory testing only on Python skills.

## Findings Throughput

For evaluating throughput, we try to execute skills in parallel. In the following example log

```text
241023_155316.286_run.log
```

```text
Evaluating: cmds=cmds/p_execute_all.cmds hash=c43cb5d37cabddd93e11c576235223e0bd695992
a  file_name=logs/241023_155316.286_run.log date=2024-10-23 15:53:16.286000
   brand=Apple M3 Pro
   arch=ARM_8  cores=cores=12 mem_total(GB)=36 mem_available(GB)=13
   binary=/Users/peter.barth/dev/pharia-kernel/target/release/pharia-kernel
   hash of binary=9290db81ab9e6c038ca3d0f1ad825aded8003317
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

We focus on execution time when accessing 10 different skills several times.
The baseline is sequential access without additional resource consumption.
Accessing 10 skills takes 23 ms, doing that 15 times takes 320 ms.
We can also do all 150 accesses in parallel, which takes 227 ms.
Not a huge difference and none expected.
More interesting is an individual skill invocation,
which uses 1 MB of memory and takes 100 ms to complete.
It is no surprise, that sequentially accessing 10 skills 15 times takes around 16.9 seconds.
It is nice, that running 150 accesses in parallel takes only 353 ms and we measure
an increase in resident memory consumption of only 11.7 MB.
Ideal scaling would mean 100 ms, no scaling at all would mean at least 15 seconds.
353 ms appears to be very acceptable.

In the cmds file

```text
saturate.cmds
```

we increase the parallel load steadily.
For that, we use 10 skills but only use some KB additional memory during a request,
but 9 seconds execution time.
Thus, we ensure that most requests run in parallel and have not finished yet.
Pharia Kernel (Oct/24) can handle 200 requests on a Mac.
On the Mac it stops answering to requests while trying to answer 300 requests in parallel.
The Mac is not suited for throughput experiments.

```text
logs/241023_161626.638_run.log_failed
```

When executing the experiment on a small x86_64 KVM server,
there are no issues up to 900 requests in parallel.

```text
241024_160204.009_run.log
```

```text
Evaluating: cmds=cmds/saturate.cmds hash=3adee1afcc53ea464359a82e89ee18235204e802
a  file_name=logs/241024_160204.009_run.log date=2024-10-24 16:02:04.009000
   brand=AMD EPYC 7301 16-Core Processor
   arch=X86_64 cores=cores=16 mem_total(GB)=126 mem_available(GB)=124
   binary=/home/peter/dev/pharia-kernel/target/release/pharia-kernel
   hash of binary=df750779249a3a19cadff4bdf716abb45ce8fb2d
============================================================================
cmd                                       id         took(ms)   rss diff(KB)
----------------------------------------------------------------------------
p_add_rs 10                               a             1,986         67,080
execute_all JohnDoe 0 0                   a                16             48
p_execute_all JohnDoe 42 9000 10          a             9,159          5,480
p_execute_all JohnDoe 42 9000 20          a             9,300          8,620
p_execute_all JohnDoe 42 9000 30          a             9,426          8,568
p_execute_all JohnDoe 42 9000 40          a             9,618          6,872
p_execute_all JohnDoe 42 9000 50          a             9,746          7,248
p_execute_all JohnDoe 42 9000 60          a             9,857         10,416
p_execute_all JohnDoe 42 9000 70          a            10,057          9,176
p_execute_all JohnDoe 42 9000 80          a            10,201          6,800
p_execute_all JohnDoe 42 9000 90          a            10,342          9,248
10 drop_cached_skill                      a                32         -7,888
10 delete_skill                           a                16              0
stop                                      a                32       -151,516
----------------------------------------------------------------------------
Total time (ms)                           a            89,788    
Maximum rss memory (KB)                   a                          159,404
```

We use 10 Rust skills which are all executed in parallel 10, 20, ... 90 times each
(we explicitly allocate 42 KB per invocation).
Thus, we have 100, 200, ... 900 parallel executions, which are in parallel as
each execution takes 9 seconds. As no complete run takes 18 seconds (10.3 seconds
is maximum) all request run in parallel. As expected, memory consumption of a
Rust skill is moderate. Without runtime overhead the skill code alone is a mere
67 MB, which is less than 7 MB per skill. We add about 5 to 10 MB per additional
100 parallel requests to the runtime. When running 900 Rust skills in memory,
we stay below 160 MB for the entire Pharia Kernel.

It is likely, that Pharia Kernel can cope with much more requests in parallel. However,
to reliably test that, we would need a set of dedicated machines that fire a concentrated
and orchestrated series of requests to the Pharia Kernel.

# Findings Pooling allocator

There is the option to use an pooling allocator for the `wasmtime` that runs the skills.
This should improve memory management. We test it by running 900 Python skills in parallel
with no wait time but using 666 MB of heap space per invocation.
In order to accomodate for run to run variations, we run each test 10 times.

First, we allocate a single chunk of memory.
Indeed, we see on average a 6 percent reduction in runtime with the pooling allocator enabled.
Note, that there is no reduction in memory consumption, if at all a slight increase, and
none is expected.

```text
Evaluating: cmds=cmds/mem_chunks_2.cmds hash=b15aad654f80726c6b991d0293a7f058de07372e comparing 2 log files
a  file_name=241025_203025.668_run.log date=2024-10-25 20:30:25.668000
   brand=AMD EPYC 7301 16-Core Processor
   arch=X86_64 cores=cores=16 mem_total(GB)=126 mem_available(GB)=124
   binary=/home/peter/dev/pharia-kernel/target/release/pharia-kernel
   hash of binary=c8a24d0b62c2e97bff6e483671c6c9d699d8ec31
b  file_name=241025_205112.006_run.log date=2024-10-25 20:51:12.006000
   brand=AMD EPYC 7301 16-Core Processor
   arch=X86_64 cores=cores=16 mem_total(GB)=126 mem_available(GB)=124
   binary=/home/peter/dev/pharia-kernel/target/release/pharia-kernel-POOLING
   hash of binary=c8a24d0b62c2e97bff6e483671c6c9d699d8ec31
==============================================================================================
cmd                                       id         took(ms)  vs a(%)   rss diff(KB)  vs a(%)
----------------------------------------------------------------------------------------------
p_add_py 10                               a            46,673               1,922,204    
                                          b            44,510       95      1,944,056      101
execute_all Alice 300 0                   a                48                   1,088    
                                          b                41       85            248       23
p_execute_all Damian 666666 0 90          a           129,836                  43,660    
                                          b           117,576       91         42,132       97
p_execute_all Damian 666666 0 90          a           126,683                  16,564    
                                          b           105,797       84         13,428       81
p_execute_all Damian 666666 0 90          a           119,494                   8,440    
                                          b           104,839       88         11,384      135
p_execute_all Damian 666666 0 90          a           109,033                   7,256    
                                          b           112,165      103          2,492       34
p_execute_all Damian 666666 0 90          a           113,210                   1,412    
                                          b           112,335       99          1,984      141
p_execute_all Damian 666666 0 90          a           113,937                   1,600    
                                          b           107,030       94          3,744      234
p_execute_all Damian 666666 0 90          a           110,766                   2,464    
                                          b           114,809      104            456       19
p_execute_all Damian 666666 0 90          a           118,034                     488    
                                          b           106,868       91            976      200
p_execute_all Damian 666666 0 90          a           106,138                   2,840    
                                          b           104,609       99            576       20
p_execute_all Damian 666666 0 90          a           109,073                     224    
                                          b           105,155       96          2,584     1154
stop                                      a               165              -2,027,932    
                                          b               114       69     -2,045,892      101
----------------------------------------------------------------------------------------------
Total time (ms)                           a         1,203,090    
                                          b         1,135,848       94
Maximum rss memory (KB)                   a                                 2,027,932    
                                          b                                 2,045,892      101



Second, we allocate many varying chunks of memory (as this is more realistic, this is
now the default implementation of the sample skills).
If heap space is greater 300 KB, we allocate chunks of random sizes between 100 and 300 KB
until we reach the intended memory consumption.
In our experiment `mem_chunks_2.cmds`,  we have about 3333 allocations on average per
skill invocation, which consumes 666 MB per invocation.

```text
a  file_name=241027_191130.260_run.log date=2024-10-27 19:11:30.260000  
   brand=AMD EPYC 7301 16-Core Processor
   arch=X86_64 cores=cores=16 mem_total(GB)=126 mem_available(GB)=124
   binary=/home/peter/dev/pharia-kernel/target/release/pharia-kernel
   hash of binary=c8a24d0b62c2e97bff6e483671c6c9d699d8ec31
b  file_name=241027_205008.554_run.log date=2024-10-27 20:50:08.554000  
   brand=AMD EPYC 7301 16-Core Processor
   arch=X86_64 cores=cores=16 mem_total(GB)=126 mem_available(GB)=124
   binary=/home/peter/dev/pharia-kernel/target/release/pharia-kernel-POOLING
   hash of binary=c8a24d0b62c2e97bff6e483671c6c9d699d8ec31
==============================================================================================
cmd                                       id         took(ms)  vs a(%)   rss diff(KB)  vs a(%)
----------------------------------------------------------------------------------------------
p_add_py 10                               a            49,495               1,842,988         
                                          b            46,763       94      1,924,640      104
execute_all Alice 300 0                   a                44                   1,140         
                                          b                40       91          1,028       90
p_execute_all Damian 666666 0 90          a           288,154                  42,824         
                                          b           292,189      101         47,428      111
p_execute_all Damian 666666 0 90          a           285,640                  14,248         
                                          b           288,394      101         16,944      119
p_execute_all Damian 666666 0 90          a           289,745                  10,660         
                                          b           287,569       99          7,820       73
p_execute_all Damian 666666 0 90          a           286,560                   6,776         
                                          b           292,845      102          9,660      143
p_execute_all Damian 666666 0 90          a           291,780                  12,024         
                                          b           289,966       99          1,144       10
p_execute_all Damian 666666 0 90          a           290,305                  17,944         
                                          b           298,499      103          1,188        7
p_execute_all Damian 666666 0 90          a           290,890                   3,948         
                                          b           292,053      100          1,944       49
p_execute_all Damian 666666 0 90          a           287,924                   2,680         
                                          b           289,868      101            236        9
p_execute_all Damian 666666 0 90          a           288,679                   6,080         
                                          b           291,718      101           -128       -2
p_execute_all Damian 666666 0 90          a           292,785                   3,168         
                                          b           288,347       98          1,144       36
stop                                      a               164              -1,984,160         
                                          b               216      132     -2,033,452      102
----------------------------------------------------------------------------------------------
Total time (ms)                           a         2,942,165         
                                          b         2,958,467      101
Maximum rss memory (KB)                   a                                 1,984,160         
                                          b                                 2,033,452      102
```

The results here are not so favorable for pooling allocations, one could call it a draw.
It is no surprise that the runtime increases, as allocating more than 3,000 chunks of
memory costs time compared to allocating one chunk. Maybe pooling allocation can be
helpful for many huge chunks of memory.

In addition, the pooling allocation consumes insane amounts of virtual memory.
`6,298,503,336 KB` which is over 6 TB and most likely due to the fact, that
the pooling allocator uses the upper parts (> 48 bits) of the pointers for tagging.
While this is no problem on real hardware or KVM machines, it might be an issue
with docker containers.
Note that the default allocator only uses `28,353,816 KB` virtual memory,
which is a mere 28 GB.
Per default, the pooling allocator is disabled.
