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

