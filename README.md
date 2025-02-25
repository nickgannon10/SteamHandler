---

Things appear to come in 400 MS bursts

Substantia Nigra: A Consciousness Research Fund
[] Fund 1: A heroes Journey

1. We are going to figure out how to create a test sweet to test elements in isolation.

- obligatory is there anything blocking? is there any improvements that we shiould obvs make

- raydium-lp-v4 should be conditioned on pumpfun withdraw

```
> `#![allow(unused_variables)]`

>#[tokio::main(flavor = "multi_thread")]
Enables multi-threaded execution (Rust can use real OS threads).
Python's asyncio cannot do this due to the GIL.
>mpsc::channel(500)
Equivalent to Python’s asyncio.Queue(maxsize=500).
Stores incoming trade messages for processing.
>tokio::spawn()
Spawns 50 async worker tasks, just like Python’s asyncio.create_task().
>rx.recv().await
Waits for a new trade message from the queue, like await queue.get() in Python.
>Worker threads (worker_threads = 8)
Unlike Python, Rust can actually distribute workers across CPU cores.

```

- global worker pool, shared by all jobs and background tasks
- Figre out how to automate load testing, so we can just run a test and see when, where, and why we break
- error handling, callbacks, graceful shutdowns, retry logic, ping thing
- bake in TPS to heartbeat
- log time to connection
- create a global connection manager
- add something like 1) both created at's and 2) delta between created at and global inserts
- things like group inserts accross all files
- env_logger::init() enables structured logging, which can be controlled with RUST_LOG

# Commands:

> `DATABASE_URL="postgres://mluser:mlpassword@localhost:5432/mldb" cargo run`
> I want logs and elastic search in here.
> log stash?

Chat GPT threads:

- First Rust Deep Research : https://chatgpt.com/c/67b8bd17-c1f0-8013-9d34-8f431b6c17eb
  - `Event Loop Bottleneck Detection & Monitoring`
  - `PostgreSQL & Redis Performance Bottlenecks`
    `Batching in all things everywhere`

```
SjZVSLBuTvCbMWyXpL5HANPv3Cp9z8P3fsiiGupAyqv4LRR22jMZ3QnTRNCQG71j6mgEr9zByK7hiLwwrBak3eQ
```
