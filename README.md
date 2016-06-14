> ### The Not Rocket Science Rule Of Software Engineering
>
> Automatically maintain a repository of code that always passes all the tests.
>
> -- Graydon Hoare (summarizing Ben Elliston)

Aelita is an implementation of this rule on top of pull requests and
a traditional CI system such as Buildbot or Jenkins.
Contributors make a pull request (obviously) and
reviewers leave a magic comment like `@aelita-mergebot r+`.

This program is very experimental at this point,
but it's already in production-esque use as @cerebrum-mergebot.


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

