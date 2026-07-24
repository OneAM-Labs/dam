# Contributing to DAM

Welcome, and thank you for your interest in contributing to **DAM**!

DAM is an actively developing **Project Continuity Platform** written in Rust. Because the architecture, CLI, and internal APIs are still in their early stages and evolving rapidly, we highly value community feedback, testing, and contributions.

This document outlines the processes and expectations for collaborating on the project.

---

## Discussions vs. Issues

To keep the project organized, we strictly separate general conversations from actionable tasks.

### Use [GitHub Discussions] for:

* Asking questions
* Sharing general ideas or feedback
* Architecture discussions and design proposals
* Community conversations and troubleshooting

### Use [GitHub Issues] for:

* Reproducible bugs
* Concrete implementation tasks
* Clearly defined feature requests

---

## Before You Contribute

### Check Existing Issues and Discussions

Before opening a new issue or discussion, check existing **Issues** and **Discussions** to ensure your idea or bug hasn't already been reported or is not currently being worked on.

### Discuss Large Changes First

If you plan to make a significant architectural change or major refactor, please start a **GitHub Discussion** to coordinate with the maintainer (`oneam`) before writing code.

This helps prevent wasted effort and ensures that proposed changes align with the project's roadmap and direction.

---

## Local Development Workflow

Contributors do not receive direct write access to the repository. All development follows the standard **Fork and Pull Request** workflow.

### 1. Fork and Clone

Fork the repository to your own GitHub account, then clone your fork locally:

```bash
git clone https://github.com/<your-username>/dam.git
cd dam
```

### 2. Create a Branch

Create a new branch for your work. Keep the branch name descriptive and specific to the change you're making.

For a new feature:

```bash
git checkout -b feature/your-feature-name
```

For a bug fix:

```bash
git checkout -b fix/issue-description
```

### 3. Build the Project

DAM is built using standard Rust tooling. Ensure that Rust and Cargo are installed on your system.

```bash
cargo build
```

### 4. Run Tests

As the project grows, test coverage will evolve. Always ensure that all existing tests pass before submitting a Pull Request.

```bash
cargo test
```

### 5. Run Local CI Checks

While the exact GitHub Actions CI workflows may evolve, contributors should run standard Rust formatting and linting checks locally before committing.

#### Check formatting

```bash
cargo fmt --all -- --check
```

#### Run the linter

```bash
cargo clippy -- -D warnings
```

> **Note:** If the project requires custom test harnesses or shell scripts in the future, they will be documented here. For now, rely on standard Cargo commands.

### 6. Submit a Pull Request

Push your branch to your fork:

```bash
git push origin your-branch-name
```

Then navigate to the main DAM repository on GitHub and open a **Pull Request** targeting the `main` branch.

---

## Pull Request Expectations

### Stay Focused

Pull Requests should address a single concern, bug, or feature.

Avoid unrelated changes, such as:

* Refactoring unrelated files
* Fixing typos in files unrelated to your change
* Making broad formatting changes outside the scope of your PR

### Describe Your Changes

Provide a clear summary of:

* What you changed
* Why you changed it
* How you tested it

Reference any related GitHub Issues where applicable. For example:

```text
Closes #12
```

### Review Process

Maintainers will review your Pull Request. Be open to feedback and prepared to make adjustments if requested.

---

## Commit Guidance

We do not currently enforce a strict commit convention, such as Conventional Commits. However, we ask contributors to follow standard best practices:

* Write clear, descriptive commit messages.
* Keep commits reasonably small and logically organized.
* Use the imperative mood.

For example:

```text
Add feature X
```

Instead of:

```text
Added feature X
```

---

## Reporting Bugs

If you discover a bug, please open a GitHub Issue.

A good bug report should include:

* Your operating system and Rust version
* The version of DAM you are using (`dam --version`) or the relevant commit hash
* Clear, step-by-step instructions to reproduce the issue
* The expected behavior
* The actual behavior

Providing complete reproduction details helps maintainers investigate and resolve issues more quickly.

---

## Security Guidance

If you discover a security vulnerability, such as an issue involving the secure credential manager or sensitive data handling, **do not open a public GitHub Issue**.

Instead, contact the maintainer directly or follow the instructions in the project's `SECURITY.md` file, if available, to report the vulnerability privately.

---

## Code of Conduct

Collaboration should be a positive experience for everyone. We expect all contributors to:

* Be respectful and considerate of others.
* Focus on constructive feedback rather than personal attacks.
* Gracefully accept constructive criticism.

---

## License and Licensing Implications

DAM is an open-source project.

By contributing code, documentation, or other materials to the project, you agree that your contributions will be licensed under the project's underlying license.

Please refer to the `LICENSE` file in the root of the repository for the full terms governing the use and distribution of the project.

---

Thank you for contributing to DAM and helping improve the project!
