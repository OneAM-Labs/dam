# DAM — Project Continuity Platform

**DAM** is a modern developer platform and command-line tool for tracking, snapshotting, and restoring the state of your projects over time. Created by **oneam**, DAM is written primarily in Rust.

> ⚠️ **Project Status: Early Development**
>
> DAM is currently in active, early-stage development. The architecture, command-line interface, and internal APIs are continuously evolving and may introduce breaking changes.

---

## 1. What is DAM?

DAM evolved from a standard Version Control System (VCS) into a comprehensive **Project Continuity Platform (PCP)**.

If you've ever wished you could:

* Save a checkpoint of your work
* Branch off to experiment safely
* Merge divergent work
* Restore a project to a known state
* Package an entire project for someone else

DAM is designed for exactly that.

Rather than focusing solely on tracking changes, DAM focuses on preserving the **continuity of a project**—its history, configuration, environment, files, and the context required to continue working on it.

---

## 2. The Problem DAM Solves

Traditional version control systems primarily ask:

> **"What changed?"**

DAM asks a broader question:

> **"How can this project be preserved, understood, recreated, and continued anywhere?"**

Maintaining project continuity often requires combining multiple disconnected tools for:

* Version history
* File collection rules
* Environment metadata
* Project setup
* Credential management
* Project packaging and distribution

DAM brings these capabilities together into a single, cohesive platform, treating project continuity as critical infrastructure.

---

## 3. How DAM Differs

### Unique Architecture

DAM is not built on top of Git, nor is it intended to be a reskin of Git. It uses its own model for representing and preserving project state.

### First-Class Project Packaging

Beyond tracking files, DAM can export complete projects into `.dam` archives containing:

* Project metadata
* Environment setup commands
* Provider profiles
* Project state

### Strict Allowlisting

File collection is governed by `.purities` (allowlists) and `.impurities` (blocklists), with deterministic conflict resolution.

This replaces error-prone, exclusion-only approaches with explicit rules defining what is permitted to enter a collection.

### Integrated Secrets Management

DAM includes a built-in credential manager that can interface directly with:

* The operating system's Keychain
* A local AES-256-GCM encrypted vault

Sensitive credentials are therefore managed by DAM rather than being stored as plaintext configuration values or environment variables.

---

## 4. Core Terminology

DAM uses terminology inspired by water control systems. These terms are primary concepts within DAM and should not be treated as direct equivalents of concepts from other version control systems.

| Term           | Definition                                                                                                                 |
| -------------- | -------------------------------------------------------------------------------------------------------------------------- |
| **Reservoir**  | The totality of a project's continuity, including history, configuration, and state. Contained in the `.dam` folder.       |
| **Collection** | A holding pool of files selected to be saved in the next snapshot, governed by purity rules.                               |
| **Seal**       | An immutable, preserved historical snapshot of everything currently in the collection.                                     |
| **Timeline**   | The complete chronological history of seals on the current stream.                                                         |
| **Stream**     | An independent, parallel flow of development that can be switched between.                                                 |
| **Flowinto**   | The act of shifting the active workspace into a different stream.                                                          |
| **Apply**      | Restoring workspace files to match a specific seal.                                                                        |
| **Sync**       | Pushing or pulling seal history and file objects between a local reservoir and a cloud platform, such as GitHub.           |
| **Drain**      | Clearing a section of the reservoir, such as emptying the collection area.                                                 |
| **Export**     | Packaging a seal or an entire project into a portable archive for sharing.                                                 |
| **Provider**   | A project-type profile, such as Flutter or Custom, that defines environment checks and setup commands for project exports. |

---

## 5. Architecture & the `.dam` Reservoir

At a high level, DAM operates on an immutable snapshot architecture.

When you initialize a project with `dam source`, DAM creates a hidden **`.dam` directory**, known as the **Reservoir**.

The Reservoir is the brain of the project and stores components such as:

* **Object Blobs** — The actual compressed data of tracked files.
* **Indices & Timelines** — A Directed Acyclic Graph (DAG) representing snapshot history.
* **Stream Metadata** — Information about parallel flows, including descriptions, goals, notes, and owners.
* **Vault** — A secure `vault.bin` file when using the local encrypted credential manager instead of the OS Keychain.
* **Config** — Reservoir-level configuration such as `config.toml` and credential-related metadata.

---

## 6. Installation & Setup

> **Note:** DAM is currently in early development and is written primarily in Rust. Building from source is the standard installation method.

Ensure that [Rust and Cargo](https://rustup.rs/) are installed on your system.

Clone the repository and build the CLI:

```bash
git clone <dam-repo-url>
cd dam
cargo install --path .
```

You can verify the installation and check for available CLI updates with:

```bash
dam update
```

---

## 7. Basic Usage

The following workflow demonstrates a standard DAM project lifecycle:

1. Initialize a reservoir
2. Define collection rules
3. Collect project state
4. Seal the collected state
5. View project history

### 7.1 Initialize the Reservoir

Navigate to your project directory and run:

```bash
dam source
```

This launches an interactive setup wizard for configuring your project, including options such as:

* Project name
* Conflict resolution
* Provider profiles

---

### 7.2 Define Purities

Create a `.purities` file to explicitly define which files may be collected.

DAM operates on strict allowlists by default.

Example:

```text
src/**
README.md
```

Only files matching the configured purity rules are eligible for collection.

---

### 7.3 Collect State

Scan the current directory and stage matching files into the Collection:

```bash
dam collect .
```

---

### 7.4 Seal the State

Create an immutable snapshot of the currently collected environment:

```bash
dam seal "Established core project structure"
```

---

### 7.5 View History

View the project's historical timeline:

```bash
dam timeline --graph
```

The `--graph` option can be used to display the history as an ASCII topological graph.

---

## 8. Advanced Features

### 8.1 Streams and Safe Context Switching

DAM supports independent Streams for parallel development.

Create a new stream:

```bash
dam stream create feature-login
```

Switch the active workspace to that stream:

```bash
dam flowinto feature-login
```

If unsealed files are present when switching streams, DAM can prompt you to create a temporary **Continuity Snapshot** to help prevent accidental data loss.

---

### 8.2 GitHub Synchronization

DAM can map local Streams directly to remote Git branches, such as:

```text
refs/heads/main
```

Authentication is handled through DAM's internal Credential Manager.

Push local stream history to the configured remote repository:

```bash
dam sync --action push
```

---

### 8.3 Pull Request Workflows

DAM provides an interactive workflow for browsing open pull requests on a remote repository and checking them out into isolated local Streams.

For example:

```bash
dam pr checkout 19
```

This checks out Pull Request `#19` into a dedicated Stream named:

```text
pr-19
```

The active workspace is protected from unintended modification during the process.

---

### 8.4 Sealed File Integrity Protection

When pulling remote changes through operations such as `dam apply` or `dam pr checkout`, DAM verifies the hashes of files in the working directory against the latest Seal.

If unsealed modifications are detected, the operation is halted to protect local work from being accidentally overwritten.

---

### 8.5 Secure Credential Management

DAM includes a native Credential Manager for managing sensitive credentials such as:

* GitHub Fine-Grained Personal Access Tokens
* SSH keys
* Other provider credentials

Credentials can be stored using the operating system's Keychain or in a local AES-256-GCM encrypted vault.

Create a credential interactively:

```bash
dam creds create --alias github_token
```

---

### 8.6 Exporting and Importing Projects

DAM can package a specific historical Seal or an entire project setup into a portable archive.

Supported export types include:

* `.seal` — A specific historical project state
* `.dam` — A complete project package

Project exports can use `dam.toml` to define:

* Environment setup commands
* Provider types
* Project configuration

Supported provider profiles may include:

* Flutter
* Python
* Custom

Export a project:

```bash
dam export project my-app --profile contributor
```

Import a shared project:

```bash
dam import shared-app.dam
```

---

### 8.7 Merging

> ⚠️ **Experimental**
>
> DAM's merge functionality is currently experimental and should not be considered ready for production workflows.

DAM includes an experimental three-way conflict resolution engine designed to combine divergent Streams based on their closest common ancestor.

Example:

```bash
dam merge feature-login --apply
```

---

## 9. Configuration & Rule Files

DAM uses rule files to strictly control what `dam collect` is allowed to collect.

These files can be placed throughout the project tree.

### `.purities`

`.purities` files define **allowlists**.

A file must match an applicable purity rule to be eligible for collection.

Example:

```text
src/**
README.md
```

---

### `.impurities`

`.impurities` files define **blocklists**.

They can be used to explicitly exclude files or directories from collection.

Example:

```text
build/
.env
```

Impurity rules inherit downwards through the project tree, allowing exclusions to be defined at the appropriate level.

Together, `.purities` and `.impurities` provide deterministic control over which project files become part of a Collection.

---

## 10. Contributing

Contributions from early adopters are welcome.

Because DAM's architecture and internal APIs are actively evolving, please coordinate with the maintainer, **oneam**, before beginning major refactors or architectural changes.

For detailed instructions on:

* Setting up the development environment
* Running tests
* Contributing code
* Submitting pull requests

See the repository's `CONTRIBUTING.md`.

---

## 11. License

Please refer to the repository's `LICENSE` file for information about usage and distribution terms.

---
