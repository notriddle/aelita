[![@TravisCI]][Build Status]
[![@AelitaBot]][Build Queue]
[![@Gitter]][Chat on Gitter]

> ### The Not Rocket Science Rule Of Software Engineering
>
> Automatically maintain a repository of code that always passes all the tests.
>
> -- Graydon Hoare (summarizing Ben Elliston)

Aelita is an implementation of this rule on top of pull requests and
a traditional CI system such as Travis or Jenkins.
Contributors make a pull request (obviously) and
reviewers leave a magic comment like `@aelita-mergebot r+`.

This program is very experimental at this point,
but it's already in production-esque use as @cerebrum-mergebot.


# But don't GitHub's Protected Branches already do this?

Travis and Jenkins both run the test suite on every branch after it's pushed to
and every pull request when it's opened, and GitHub can block the pull requests
if the tests fail on them. To understand why this is insufficient to get an
evergreen master, imagine this:

  * #### Pull Request \#1: Rename `bifurcate()` to `bifurcateCrab()`

    Change the name of this function, as well as every call site that currently
    exists in master. I've thought of making it a method on Crab instead of on
    Sword, but then it would be `bifurcateWithSword()`, which hardly seems like
    an improvement.

  * #### Pull Request \#2: `bifurcate()` after landing, in addition to before

    Adds another call to `bifurcate()`, to make sure it gets done even if we
    skip the pre-landing procedure.

When both of these pull requests are sitting open in the backlog, they will
both be tested against master. Assuming they both pass, GitHub will happily
present the Big Green Merge Button. Once they both get merged master will
go red (Method `bifurcate()` not found).

In addition to the testing requirements, GitHub can also be set to block pull
requests that are not "up to date" with master, meaning that problems like this
can show up. This fixes the problem, by requiring that master only contain a
snapshot of the code that has passed the tests, but it requires maintainers to
manually:

 1. "Update the pull requests," merging them onto master without changing
    master itself
 2. Wait for the test suite to finish
 3. Merge the pull request when it's done, which is a trivial operation that
    can't break the test suite thanks to step 1

And it has to be done for every pull request one at a time.

This is essentially the process that aelita automates. Instead of merging,
you add reviewed pull requests to a "merge queue" of pull requests that are
tested against master by copying master to a staging branch and merging into
that. When the status of staging is determined (either pass or fail), aelita
reports the result back as a comment and merges staging into master if it was
a pass. Then it goes on to the next one.

Note that aelita is not a replacement for Jenkins or Travis. It just implements
this workflow.


## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.


[@TravisCI]: https://travis-ci.org/AelitaBot/aelita.svg?branch=master
[Build Status]: https://travis-ci.org/AelitaBot/aelita
[@AelitaBot]: https://img.shields.io/badge/passing-automatically-FF69b4.svg
[Build Queue]: http://aelita-mergebot.xyz/AelitaBot/aelita
[@Gitter]: https://img.shields.io/gitter/room/aelitabot/aelitabot.svg?maxAge=2592000
[Chat on Gitter]: https://gitter.im/aelitabot/Lobby
