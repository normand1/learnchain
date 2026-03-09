# 2026-03-08: Deep-Dive Markdown Structure and Embedded Quiz

Status: Accepted

## Context

LearnChain deep dives were previously narrative-only markdown artifacts. The standalone quiz flow already had a stable grouped question structure, but that structure was not present inside saved deep-dive documents.

The new requirement is that every generated deep dive must include an embedded quiz while preserving the existing deep-dive markdown contract:

- the file remains a single `.md` artifact with TOML front matter
- the quiz should reuse the existing grouped quiz structure instead of introducing a second format
- optional deep-dive sections should still respect `DeepDiveSectionsConfig`

## Decision

The final deep-dive markdown artifact now has two layers:

1. TOML front matter that stores durable metadata for history scanning and exports
2. a markdown body assembled in a fixed order, with the quiz rendered as a first-class section inside the document

The LLM response now carries `quiz_groups`, which reuse the existing quiz group and question types:

- `KnowledgeResponse`
- `QuizItem`
- `QuizOption`

This keeps standalone quiz exports and embedded deep-dive quizzes aligned at the data-shape and markdown-shape levels.

## Final File Shape

Every saved deep dive is written as:

```md
+++
artifact_type = "session_deep_dive"
title = "..."
generated_at = "..."
session_source = "..."
session_id = "..."
session_timestamp = "..."
session_date = "..."
project_name = "..."
project_cwd = "..."
source_file = "..."
referenced_url_count = 0
reviewed_url_count = 0

[session_analytics]
total_tool_calls = 0
successful_tool_calls = 0
failed_tool_calls = 0
unknown_outcome_tool_calls = 0
mcp_tool_calls = 0
external_lookup_calls = 0
adjust_course_count = 0
external_resources = []
adjustments = []
+++

# {title}

...
```

The front matter is always followed by a blank line and then the rendered markdown body.

## Markdown Body Order

The markdown body is rendered in this order:

1. `# {title}`
2. `## Session Metadata`
   Rendered only when `deep_dive_sections.session_metadata` is enabled.
3. `## Session Analytics`
   Rendered when analytics data is present. This is not controlled by the deep-dive section toggles.
4. `## Goal`
   Rendered only when `deep_dive_sections.goal` is enabled.
5. `## What Was Accomplished`
   Rendered only when `deep_dive_sections.accomplishments` is enabled.
6. `## Interesting or Unexpected Learnings`
   Rendered only when `deep_dive_sections.interesting_learnings` is enabled.
7. `## Teaching Narrative`
   Rendered only when `deep_dive_sections.teaching_narrative` is enabled.
8. `## Quiz`
   Always rendered.
9. `## Reviewed External Sources`
   Rendered only when `deep_dive_sections.reviewed_external_sources` is enabled.
10. `## Referenced URLs`
    Rendered only when `deep_dive_sections.referenced_urls` is enabled.

## Section Details

### Session Metadata

When enabled, this section is a flat bullet list with:

- session source
- session date
- session id
- project name
- working directory
- source file

### Session Analytics

When analytics are present, this section contains:

- top-level summary bullets for tool-call outcomes and adjustment counts
- `### External Resources`
- `### Adjustments Detected`

If either subsection has no entries, it renders `- None`.

### Goal

This section is a single paragraph.

### What Was Accomplished

This section is a bullet list. If the model returns no accomplishments, the renderer falls back to `- None provided.`

### Interesting or Unexpected Learnings

This section is also a bullet list with the same `- None provided.` fallback behavior.

### Teaching Narrative

This section is rendered as markdown blocks rather than a plain bullet list. The expected shape is:

- short subsections
- `###` subheadings inside the body
- short paragraphs separated by blank lines

If no teaching narrative is returned, the body renders `No teaching narrative was provided.`

### Quiz

This section is always present and is the embedded version of LearnChain's existing quiz markdown structure.

If the model returns no quiz groups, the section renders:

```md
## Quiz

No quiz questions were generated.
```

Otherwise the section uses the same layout as standalone quiz exports, but with headings shifted down one level so the deep-dive section remains the owning `##` block:

- quiz group heading: `### {knowledge_type_group}`
- fallback quiz group heading: `### Knowledge Group {n}`
- optional language line: `- Language: {knowledge_type_language}`
- optional group summary paragraph
- per-question heading: `#### Question {n}`
- answer options as flat bullets
- correct answer marked inline with ` (correct)`
- optional `Resources:` subsection followed by bullet links or paths

The embedded quiz therefore has this shape:

```md
## Quiz

### {knowledge_type_group}
- Language: {knowledge_type_language}

{group summary}

#### Question 1
{question text}

- {option a}
- {option b} (correct)
- {option c}

Resources:
- {resource 1}
- {resource 2}
```

### Reviewed External Sources

When enabled, each reviewed source renders as:

```md
### {url}
{summary}

Why it mattered: {why_it_matters}
```

If no reviewed sources are available, the section renders `No external sources were reviewed during generation.`

### Referenced URLs

When enabled, this section renders either:

- a flat bullet list of session URLs

or:

- `No external URLs were referenced in the session.`

## Consequences

- Every deep dive is now a self-contained learning artifact with both explanation and assessment.
- The embedded quiz stays structurally aligned with standalone quiz rendering.
- Deep-dive section toggles continue to control the narrative sections only; the quiz is intentionally mandatory.
