# Contributing to PhariaKernel

Thanks for your interest in the PhariaKernel, the serverless AI platform. Check out the [README](README.md) for more information on how to get started.

## New Features

We welcome external contributions. Before putting in major refactorings or features, we recommend opening an issue to discuss the design and get early feedback.
If you are unsure if an idea aligns with our strategic direction, please also discuss in an issue.
Seeking approval for ideas early on avoids frustration on both sides.

## Bug Reports

- If the bug is a *security vulnerability*, do not open an issue, but instead report it to support@aleph-alpha.com.
- Ensure the bug was not already reported by searching through the existing issues.
- If you're opening a new issue, be sure to include a title and clear description, and as much relevant information as possible. If possible, add a minimal reproducible example or a failing test case.

## Coding Guidelines

In this repository we stick to Conventional Commits. See: <https://www.conventionalcommits.org/en/v1.0.0/>.
PRs should be small, focused, and incremental. Consider adding a description of your change in the PR body.
Commits should be small and change one thing. Make sure your code is formatted, linted, and all the tests pass.
Make sure abstractions introduced are easily testable. If they are not, this likely indicates a design issue.
The Kernel uses an actor model. Outside of tests, we avoid shared mutable state.

## Testing

Every change should be covered by tests. Try to stick to the [Given-When-Then](https://martinfowler.com/bliki/GivenWhenThen.html) pattern. Test names should be clear and descriptive.
Make sure you understand the [naming conventions](https://martinfowler.com/articles/mocksArentStubs.html#TheDifferenceBetweenMocksAndStubs) around test doubles if introducing new ones.

Thanks - the PhariaKernel team ❤️