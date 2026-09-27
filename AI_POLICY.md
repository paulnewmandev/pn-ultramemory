# Policy on AI-assisted contributions

Coding assistants are useful and this project does not forbid them. It does insist that a **human
is accountable** for every line that lands, because that is the only thing that makes a review
meaningful.

## The rules

1. **You are the author.** When you open a pull request, you are stating that you understand the
   change, that you believe it is correct, and that you can answer questions about it. "The tool
   wrote it" is not an answer to a review comment.
2. **No machine co-authors.** Do not add `Co-Authored-By` trailers naming an assistant, a model or
   a vendor, and do not add "generated with" lines to commit messages or pull request
   descriptions. Commit authorship records the person who is responsible.
3. **Disclosure is optional and welcome.** If you want to say that a change was drafted with
   assistance, add a single `Assisted-by: <tool>` trailer. It carries no weight in review, and its
   absence implies nothing.
4. **The bar is the same for everyone.** Every change passes the gates in
   [CONTRIBUTING.md](CONTRIBUTING.md): formatting, lints, tests, documentation on every item,
   SPDX headers and the dependency policy. A change that cannot pass them does not land, whoever
   or whatever drafted it.
5. **Do not paste code you have no right to contribute.** Everything you submit is licensed under
   [Apache-2.0](LICENSE) by your sign-off (see the Developer Certificate of Origin in
   [CONTRIBUTING.md](CONTRIBUTING.md)). If you cannot make that statement about a piece of code,
   do not submit it.
6. **Review what you send.** Plausible-looking code that nobody read is the failure mode this
   policy exists to prevent. Read the diff before you open the pull request.

## What we do not do

We do not run detectors, we do not ask how a change was produced, and we do not treat a suspicion
of assistance as a defect. Correctness, clarity and tests are what we look at.

## Why this exists

This tool stores what coding agents learn and hands it back to them later. If the project itself
were built without anyone accountable for its behavior, none of its claims about provenance,
confidence and trust would be worth anything.
