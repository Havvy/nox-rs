## Differences ##

* Written in Rust instead of Python
* Query argument is required. This may be changed to `nox --query a`.
* It's an error if the manifest.json file isn't there. [Temporary bug]
* No installing into a shell thing.
* If empty input is given, assume nothing wants to be installed.