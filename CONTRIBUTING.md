When you submit pull requests,
please do not include merge commits or broken code in any of your commits.
To remove such things, follow this [squashing your commits howto].

All source code is limited to 80 characters wide at maximum,
and is indented by two spaces.
In documentation (like this file), we use [semantic newlines].

As a special exception, you do not need to line break in the middle of a URL
or other opaque constant that's just being assigned to a reasonably-sized name.

[squashing your commits howto]: http://gitready.com/advanced/2009/02/10/squashing-commits-with-rebase.html
[semantic newlines]: http://rhodesmill.org/brandon/2012/one-sentence-per-line/
[75 characters is at the high end of optimal legibility]: http://baymard.com/blog/line-length-readability

Architecture
------------

  * A "pipeline" is a queue of approved pull requests,
    and the state of one that is currently being tested. \
    ![Each pull request, to be tested,
    gets merged, tested, then finally master is fast-forwarded to the
    aforementioned merge commit]
    (https://raw.githubusercontent.com/AelitaBot/aelita/0a4f867052fd0464855620570d65688ec06df8cc/pipeline_state_machine.png)
  * A pipeline is also associated with a frontend User Interface (UI),
    a backend Continuous Integration (CI) service,
    and a Version Control System (VCS),
    which are collectively referred to as the Workers.
  * A single Worker may be associated with more than one pipeline;
    they are expected to keep track of any pipeline-specific configuration.
    For example, GitHub allows each pipeline to have its own repository,
    as does the Git backend itself.
    Jenkins, in contrast, associates each pipeline with a job.
  * This data structure is set up by main based on the contents of a config
    file. \
    ![Pipelines are owned by the main thread directly, as are the workers.
    Pipelines borrow the corresponding workers]
    (https://raw.githubusercontent.com/AelitaBot/aelita/0a4f867052fd0464855620570d65688ec06df8cc/ownership.png)
