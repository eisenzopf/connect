# Connect developer documentation

Start at [Build with Connect](index.html), choose the matching published release
or development source, and follow the topic you need.

<!-- CONNECT:NAV:START -->
[Install](start/index.html#install) · [First message](start/index.html#first-message) · [Voice and video](reference/index.html#voice) · [Handoff](reference/index.html#handoff) · [Parley tutorial](tutorials/parley.html) · [Concepts](concepts/index.html) · [Hosting](hosting/index.html) · [SDK and HTTP reference](reference/index.html) · [Contribute](../CONTRIBUTING.md) · [Engineering](engineering/index.html)
<!-- CONNECT:NAV:END -->

The published local macOS arm64 package and current Gate 6 source have different
API generations. The entry pages label source guides, published instructions,
target contracts and unfinished application/hosting qualification separately.
Plans and historical proof live under [Engineering](engineering/index.html);
their files and stable URLs remain available.

The [documentation structure](DOCUMENTATION.md) maps these surfaces to their
owned follow-up tasks. [navigation.json](navigation.json) is the single source
for shared links, terminology and the reference index. Regenerate or check it:

```sh
python3 scripts/generate-developer-navigation.py
python3 scripts/generate-developer-navigation.py --check
```

Run those commands from the repository root. They check local targets and HTML
anchors, not remote websites, SDK execution or runtime qualification.
