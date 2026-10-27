# Developer documentation structure

G6-081 defines the information structure. Its entry pages are implemented locally;
publication and complete installed documentation qualification remain separate.

The developer path is **install → first message → concepts → media/transition
guides → progressive Parley checkpoints → reference or hosting → contribution**.
Start with a buildable goal and its actual version. Plans and historical proof
are supporting engineering material, not the first reading assignment.

| Surface | Responsibility | Current scope / later completion |
| --- | --- | --- |
| Repository README and `docs/index.html` | Goals, matching version, direct topic navigation | Shared navigation generated from `navigation.json`; source capabilities carry current limits |
| `docs/start/` | Published first conversation and current source entry | Published demo remains explicitly versioned; clean installed Gate 6 quickstart is G6-082 |
| `docs/concepts/` | Shared vocabulary and model sources | Full contract-traced diagrams/lifecycle examples are G6-083 |
| `docs/tutorials/parley.html` | Progressive application path | Scenario/fixtures are G6-071; executed installed checkpoints require G6-079/G6-084 |
| `docs/reference/` | SDK, generated HTTP contracts and UCTP discovery | Standalone rendered references and installed snippet/curl execution are G6-085 |
| `docs/hosting/` | Local configuration, target platform and truthful readiness | Complete Linux/provider guide requires G6-086 prerequisites |
| `CONTRIBUTING.md` and connector guide | Contribution entry and ownership | Complete installed contribution walkthrough is G6-087 |
| `docs/engineering/` | Plans, decisions, release history and scoped proof | Earlier entry content is retained here with repaired relative links; existing plan/evidence paths remain stable |

`navigation.json` defines topic IDs, labels, routes and scope descriptions. The
same generator renders navigation into repository Markdown, the website entry
pages and the generated-contract reference index. It also renders the terminology
table. Change the map first; `--check` refuses drift or missing local targets.
Page-specific prose must use these terms and cite the canonical contract when
explaining semantics. The current model decision owns lifecycle terminology;
an earlier baseline is historical when its rules differ.

Distinguish **published v0.1**, **current development source**, **target contract**,
**fixture**, and **unqualified profile**. A successful HTTP request is not media
readiness; an ACK is not a completed external effect; model output cannot approve
an application action. Local source checks never imply installed, Linux, remote
or provider results. New entry pages must preserve those boundaries.

The public website change set and capability story are G6-089. G6-090 owns full
internal/external link, accessibility, responsive-layout, installed-example and
claim verification. The G6-081 local link/layout checks do not substitute for
those tasks or any Gate 6 acceptance criterion.
