# How Solana Gossip Really Works: A Byte-Level Journey Into the Devnet

> A complete walkthrough of building a working gossip client from scratch, debugging wire format mismatches byte-by-byte, and finally talking to the Solana devnet entrypoint.

---

## Table of Contents

1. [The Dream](#1-the-dream)
2. [The Gossip Protocol in 60 Seconds](#2-the-gossip-protocol-in-60-seconds)
3. [Our Starting Point](#3-our-starting-point)
4. [The Debugging Journey](#4-the-debugging-journey)
   - [Phase 1: Ping Works, PullRequest Gets Ignored](#phase-1-ping-works-pullrequest-gets-ignored)
   - [Phase 2: Chasing the Wrong Clue (Version struct)](#phase-2-chasing-the-wrong-clue-version-struct)
   - [Phase 3: The CrdsValue hash Trap](#phase-3-the-crdsvalue-hash-trap)
   - [Phase 4: The REAL Root Cause — ContactInfo Serialization](#phase-4-the-real-root-cause--contactinfo-serialization)
5. [Byte-by-Byte Wire Format Analysis](#5-byte-by-byte-wire-format-analysis)
   - [How Bincode Serializes Things](#how-bincode-serializes-things)
   - [The Version Struct: 17 bytes → 12 bytes](#the-version-struct-17-bytes--12-bytes)
   - [The ContactInfo Struct: 307 bytes → 65 bytes](#the-contactinfo-struct-307-bytes--65-bytes)
   - [The CrdsValue: 32 bytes of hash poison](#the-crdsvalue-32-bytes-of-hash-poison)
6. [The Complete Devnet Conversation](#6-the-complete-devnet-conversation)
   - [Message 1: Ping (132 bytes)](#message-1-ping-132-bytes)
   - [Message 2: Pong (132 bytes)](#message-2-pong-132-bytes)
   - [Message 3: PullRequest (1232 bytes)](#message-3-pullrequest-1232-bytes)
   - [Message 4: Entrypoint's Ping (132 bytes)](#message-4-entrypoints-ping-132-bytes)
   - [Message 5: PullResponse (505-1232 bytes)](#message-5-pullresponse-505-1232-bytes)
7. [The Complete File-by-File Change Log](#7-the-complete-file-by-file-change-log)
8. [What We Still Get Wrong (and Why It Doesn't Matter)](#8-what-we-still-get-wrong-and-why-it-doesnt-matter)
9. [How to Run It Yourself](#9-how-to-run-it-yourself)
10. [All Reference Files in Agave Source](#10-all-reference-files-in-agave-source)

---
