---

Substantia Nigra: A Consciousness Research Fund
[] Fund 1: A heroes Journey

- Figre out how to automate load testing, so we can just run a test and see when, where, and why we break

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
