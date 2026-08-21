# Codebase Memory: implementation brief (document of record)

Owner mandate, 2026-08-21: **top-priority feature.** Build a reusable
codebase-intelligence subsystem (persistent AST-derived code knowledge graph,
retrieval layer, Claude Code integration), FNDR-first but installable into
arbitrary repositories. Roadmap epic: E16 in `docs/ROADMAP-TICKETS.md`
(kickoff ticket T-1601 executes section 28 below before any code).

The owner's brief follows verbatim (its em dashes and formatting are
preserved as source material; house style applies to our documents, not to
quoted specifications).

---

You are working inside the FNDR codebase. Build a production-quality, reusable "Codebase Memory" subsystem that gives Claude Code persistent structural memory of every software repository it works with.

IMPORTANT:
Do not implement this as ordinary vector RAG.
Do not simply embed source files and retrieve top-k chunks.
The primary representation must be a persistent knowledge graph derived from the actual structure of the codebase, with optional semantic/LLM enrichment layered on top.

The goal is to create an internal equivalent of a persistent codebase-memory system: when Claude Code starts a new session, it should be able to reconstruct the architecture of a repository from a previously generated graph instead of rediscovering the entire repository through grep/read operations.

The system must be reusable across FNDR and arbitrary external repositories.

==================================================
1. CORE OBJECTIVE
==================================================

Create a subsystem with this conceptual pipeline:

    Repository
        │
        ▼
    Language-aware parser / AST analysis
        │
        ▼
    Code entities + relationships
        │
        ▼
    Persistent Code Knowledge Graph
        │
        ├── structural queries
        ├── dependency traversal
        ├── impact analysis
        ├── symbol lookup
        ├── architectural summaries
        └── semantic enrichment
        │
        ▼
    Claude Code integration
        │
        ▼
    relevant architectural context
        │
        ▼
    source-code reads only where necessary

The system should make a new Claude Code session behave as though it already understands the repository's architecture.

Do NOT rely on Claude's conversational memory for this.

The repository itself must contain/persist the machine-readable representation of its architecture.

==================================================
2. ARCHITECTURAL PRINCIPLE
==================================================

Separate the system into four layers:

LAYER A — CODE INDEXER

Responsible for turning source code into structured entities and relationships.

LAYER B — CODE KNOWLEDGE GRAPH

Persistent representation of the repository.

LAYER C — CODEBASE RETRIEVAL / REASONING

Given a natural-language question, retrieve the relevant graph neighborhood/path/subgraph.

LAYER D — CLAUDE CODE INTEGRATION

Automatically route repository-exploration tasks through the code graph before Claude performs expensive raw grep/read exploration.

The interfaces between these layers must be clean enough that the implementation can later support additional languages, storage engines, and LLMs.

==================================================
3. CODE GRAPH
==================================================

Create a typed graph.

Nodes should support at minimum:

    Repository
    Directory
    File
    Module
    Namespace
    Package
    Class
    Struct
    Interface
    Trait
    Function
    Method
    Variable
    Constant
    Enum
    Type
    DatabaseTable
    APIEndpoint
    Configuration
    Test
    Documentation
    ADR / RFC
    ExternalDependency

Relationships should support at minimum:

    CONTAINS
    DEFINES
    IMPORTS
    EXPORTS
    CALLS
    REFERENCES
    IMPLEMENTS
    EXTENDS
    INHERITS
    USES
    DEPENDS_ON
    RETURNS
    ACCEPTS
    INSTANTIATES
    OVERRIDES
    TESTS
    CONFIGURES
    EXPOSES
    READS
    WRITES
    STORES_IN
    QUERIES
    MIGRATES
    DOCUMENTS
    JUSTIFIED_BY
    REPLACED_BY

Do not create relationships merely because two files contain similar words.

Relationships should have provenance.

Every edge should contain something conceptually similar to:

    source
    target
    relationship_type
    provenance
    confidence
    source_location
    metadata

Provenance should distinguish:

    EXTRACTED
    INFERRED
    LLM_INFERRED
    AMBIGUOUS

Example:

    MemoryService
        --CALLS-->
    EmbeddingService

should be EXTRACTED if discovered from the AST.

Whereas:

    BGEEmbedding
        --JUSTIFIED_BY-->
    ADR-007

may be LLM_INFERRED or DOCUMENTATION_DERIVED.

==================================================
4. AST-BASED INDEXING
==================================================

Use AST parsing rather than regex wherever possible.

The indexer must be incremental.

On initial indexing:

    repository
        → discover files
        → identify supported languages
        → parse source
        → extract symbols
        → resolve references
        → construct graph
        → persist graph

On subsequent runs:

    git/file changes
        → identify changed files
        → reparse only affected files
        → invalidate affected graph nodes
        → recompute affected relationships
        → update graph

Do NOT rebuild the entire repository every time.

Track:

    file hash
    parser version
    schema version
    git commit
    last indexed timestamp

so stale graph state can be detected.

==================================================
5. LANGUAGE SUPPORT
==================================================

Design this around language adapters.

Create an abstraction similar to:

    LanguageIndexer

with implementations for the languages most relevant to FNDR first.

At minimum investigate support for:

    Rust
    TypeScript
    JavaScript
    Python
    C
    C++
    C#
    Java
    Go

Do not blindly implement every language if the architecture can support adapters cleanly.

The first implementation should prioritize:

    Rust
    TypeScript
    JavaScript
    Python

because those are especially relevant to FNDR and modern agentic software projects.

Use Tree-sitter or another robust parser where appropriate.

Do not build a fragile regex parser.

==================================================
6. SYMBOL RESOLUTION
==================================================

The graph is only useful if references resolve correctly.

Implement symbol resolution where practical.

For example:

    File A
       │
       └── calls Foo()
                   │
                   ▼
             File B::Foo

The graph should represent the actual target symbol rather than just storing:

    "A contains the string Foo"

Support:

    local symbols
    imported symbols
    module paths
    namespaces
    class methods
    interfaces/traits
    external packages

When resolution cannot be determined confidently, preserve the ambiguity rather than fabricating a relationship.

==================================================
7. GRAPH STORAGE
==================================================

Do not prematurely couple the system to a hosted graph database.

The default should work locally.

Evaluate appropriate options such as:

    SQLite
    JSON
    embedded graph storage
    LanceDB only where vector search is actually useful

The system should produce a portable representation.

Something conceptually like:

    .fndr/codegraph/
        graph.db
        graph.json
        metadata.json
        summaries/
        cache/
        schema/

Avoid committing massive generated artifacts unless there is a deliberate reason to do so.

The system should support:

    local-only operation
    deterministic indexing
    no cloud dependency

This is important because FNDR's architecture strongly favors local/private processing.

==================================================
8. GRAPH QUERY ENGINE
==================================================

Implement a query layer that supports questions like:

    What calls X?

    What does X depend on?

    What depends on X?

    What files implement interface X?

    What is the path between A and B?

    What is the blast radius of changing X?

    Where is X defined?

    What APIs eventually reach X?

    Which tests cover X?

    What database tables does X read/write?

    What modules are part of this subsystem?

    What are the most connected nodes?

    What are the architectural communities/modules?

The query engine should return a compact subgraph rather than dumping the entire graph.

Example:

    query("What depends on EmbeddingService?")

should produce something conceptually like:

    EmbeddingService
        ↑
        ├── MemoryIndexer
        │     ↑
        │     └── MemoryPipeline
        │
        ├── SearchService
        │
        └── BatchReindexer

along with:

    node metadata
    file paths
    source locations
    relationship types
    confidence/provenance

==================================================
9. NATURAL LANGUAGE GRAPH RETRIEVAL
==================================================

Implement a natural-language interface.

For:

    "If I change the embedding dimension, what breaks?"

the system should identify relevant entities:

    embedding
    vector
    schema
    index
    LanceDB
    retrieval
    serialization
    migration

Then traverse the graph around those entities.

The retrieval algorithm should combine:

    exact symbol lookup
    lexical matching
    graph traversal
    dependency paths
    relationship types
    optional semantic retrieval

Semantic embeddings may be used as a SECONDARY retrieval mechanism.

They must not replace the structural graph.

The final context should look like:

    Relevant architectural path:

    BGEEmbedding
        ↓
    EmbeddingService
        ↓
    MemoryIndexer
        ↓
    MemoryChunk
        ↓
    LanceDB vector schema
        ↓
    RetrievalService

    Relevant files:
        src/embedding/...
        src/memory/indexer.rs
        src/memory/chunks.rs
        src/search/retrieval.rs

    Relevant relationships:
        ...

This should then guide Claude toward the correct files.

==================================================
10. ARCHITECTURAL SUMMARIZATION
==================================================

Generate higher-level architecture summaries from the graph.

Produce things conceptually similar to:

    GRAPH_REPORT.md

Include:

    repository overview
    major modules
    dependency communities
    highly connected components
    architectural entry points
    core services
    external integrations
    database/storage boundaries
    API boundaries
    suspicious coupling
    circular dependencies
    isolated components
    important architectural paths

The report should be regenerated incrementally when necessary.

Do not make the report the primary source of truth.

The graph is the source of truth.

The report is a human/LLM-friendly projection of the graph.

==================================================
11. "WHY" / ENGINEERING MEMORY
==================================================

The system must preserve architectural rationale where available.

Search for:

    ADRs
    RFCs
    README files
    architecture documents
    design documents
    comments
    TODOs
    migration notes
    commit messages

Represent relationships such as:

    BGEEmbedding
        --REPLACED_BY-->
    NewEmbeddingModel

and:

    NewEmbeddingModel
        --JUSTIFIED_BY-->
    ADR-012

The objective is to preserve both:

    WHAT the code does

and:

    WHY the code was designed that way.

Do not invent rationale.

Clearly distinguish extracted rationale from inferred rationale.

==================================================
12. CLAUDE CODE INTEGRATION
==================================================

This is critical.

Create a Claude Code integration so Claude naturally uses the graph.

The integration should provide:

    skill/instructions
    CLI commands
    optional MCP server
    Claude hooks where appropriate

The desired behavior is:

    Claude starts session
        ↓
    discovers codebase graph
        ↓
    understands repository topology
        ↓
    user asks architectural/code question
        ↓
    graph query occurs
        ↓
    relevant subgraph returned
        ↓
    Claude reads only relevant source files
        ↓
    Claude performs task

Do not require the user to manually tell Claude:

    "Use the code graph."

The integration should make this the default behavior.

==================================================
13. CLAUDE TOOL-USE POLICY
==================================================

Create a Claude Code skill/instruction set that establishes:

Before performing broad repository exploration:

    1. Query the code graph.
    2. Identify relevant nodes.
    3. Traverse dependencies/callers/implementations.
    4. Identify likely files.
    5. Read those files.
    6. Use grep/search only when the graph cannot answer the question.

For example:

USER:
    "Where is authentication handled?"

Claude should prefer:

    codegraph query "authentication"
        ↓
    identify AuthService
        ↓
    traverse CALLS / EXPOSES / MIDDLEWARE
        ↓
    identify relevant files
        ↓
    Read files

rather than:

    grep -R "auth" .

For implementation tasks:

    graph query
        ↓
    impact analysis
        ↓
    relevant source
        ↓
    modify code
        ↓
    update graph

==================================================
14. PRE-TOOL HOOKS
==================================================

Investigate Claude Code's hook system and implement safe hooks that can intercept broad repository exploration.

The system should recognize situations where Claude is about to perform:

    broad grep
    broad file search
    recursive repository exploration

and provide graph context first.

Do NOT aggressively block Claude from using normal tools.

The graph should assist Claude, not cripple it.

If the graph cannot answer something, Claude must still be able to use:

    grep
    rg
    find
    git
    file reads
    other tools

The desired policy is:

    GRAPH FIRST
    SOURCE SECOND
    RAW SEARCH WHEN NECESSARY

not:

    GRAPH ONLY

==================================================
15. MCP INTERFACE
==================================================

Where practical, expose the graph through MCP.

Provide tools conceptually equivalent to:

    codebase_search
    codebase_query
    codebase_symbol
    codebase_dependencies
    codebase_dependents
    codebase_callers
    codebase_callees
    codebase_path
    codebase_impact
    codebase_context
    codebase_summary

Example:

    codebase_impact(
        symbol="EmbeddingService"
    )

should return:

    direct dependents
    indirect dependents
    callers
    storage dependencies
    tests
    configuration
    relevant files

The output must be optimized for LLM consumption.

Do not return enormous JSON payloads.

==================================================
16. CONTEXT COMPRESSION
==================================================

The entire purpose is to reduce context consumption.

Do not send entire files to Claude when the graph can first narrow the search.

Create compact representations:

    Node:
        name
        type
        file
        line
        signature

    Edge:
        source
        relationship
        target

    Path:
        A → B → C → D

The graph retrieval layer should aggressively remove irrelevant nodes.

A useful context hierarchy is:

    Level 1:
        architecture summary

    Level 2:
        relevant subgraph

    Level 3:
        relevant symbols

    Level 4:
        source code

Claude should descend through these levels only as necessary.

==================================================
17. SESSION CONTINUITY
==================================================

The graph must survive Claude Code sessions.

Session 1:

    Claude learns architecture
    Claude modifies retrieval system
    graph updates

Session ends.

Session 2:

    Claude queries graph
    immediately understands modified architecture

Session 3:

    Claude queries graph
    sees dependencies and historical rationale

The graph must therefore be external to the LLM's context window.

==================================================
18. GIT INTEGRATION
==================================================

Use Git metadata where available.

Track:

    current commit
    changed files
    authors
    commit timestamps
    relevant commit messages

Optionally connect:

    symbol
        → commit
        → rationale

Support:

    "Why did this code change?"

by finding relevant commits/documentation.

Do not assume commit messages are authoritative.

Mark them as historical evidence.

==================================================
19. CODEBASE CHANGE DETECTION
==================================================

Implement:

    graphify index

    graphify update

    graphify status

or equivalent commands.

Expected behavior:

    git diff
        ↓
    changed files
        ↓
    affected symbols
        ↓
    affected edges
        ↓
    incremental graph update

The system should be fast enough to run frequently.

==================================================
20. GRAPH VALIDATION
==================================================

Build tests for graph correctness.

Test:

    symbol extraction
    relationship extraction
    symbol resolution
    incremental updates
    deleted files
    renamed files
    ambiguous references
    cross-module dependencies
    cross-language references where supported

Create small fixture repositories.

For each fixture, define expected graph relationships.

Example:

    a.py calls b.py::foo

Expected:

    a.foo
        --CALLS-->
    b.foo

Do not rely exclusively on LLM-generated tests.

==================================================
21. FNDR INTEGRATION
==================================================

Integrate this into FNDR without contaminating the existing memory/search architecture.

Keep "Codebase Memory" conceptually separate from FNDR's user-memory system.

FNDR currently deals with memories such as:

    screenshots
    OCR
    text chunks
    embeddings
    semantic retrieval
    memory intelligence

Do not merge those concepts.

Instead:

    FNDR User Memory
        = information about the user's digital activity

    FNDR Codebase Memory
        = structural knowledge about software repositories

They may share infrastructure where appropriate, but their schemas and semantics must remain distinct.

==================================================
22. SECURITY / PRIVACY
==================================================

Default to local processing.

Do not upload source code to external services.

Do not send repository contents to an LLM merely to build the basic graph.

AST extraction should be deterministic and local.

If optional LLM enrichment is implemented:

    make it explicit
    make it configurable
    identify exactly what data leaves the machine
    support completely local operation

Never expose secrets discovered in:

    .env
    credentials
    private keys
    tokens
    certificates
    secret configuration

These files should be excluded from indexing.

==================================================
23. PERFORMANCE
==================================================

Measure:

    initial indexing time
    incremental indexing time
    graph query latency
    memory usage
    graph size
    Claude context reduction

The system should scale to:

    small repositories
    medium monorepos
    large repositories

Avoid O(N²) graph operations where possible.

Cache:

    parsed ASTs
    symbol tables
    resolution results
    graph fragments

==================================================
24. CLI
==================================================

Provide a clean CLI.

Something conceptually like:

    fndr codegraph init

    fndr codegraph index

    fndr codegraph update

    fndr codegraph status

    fndr codegraph query "what calls EmbeddingService?"

    fndr codegraph impact "EmbeddingService"

    fndr codegraph path "MemoryService" "LanceDB"

    fndr codegraph symbol "MemoryChunk"

    fndr codegraph summary

    fndr codegraph visualize

Names can change based on the existing FNDR CLI architecture.

Follow existing FNDR naming conventions.

==================================================
25. VISUALIZATION
==================================================

Provide a graph visualization for developers.

At minimum support:

    node inspection
    relationship inspection
    filtering
    module/community grouping
    path highlighting
    dependency direction

Do not attempt to visualize every node simultaneously.

The visualization should support exploration of:

    architecture
    dependencies
    blast radius
    subsystem boundaries

==================================================
26. IMPORTANT DESIGN CONSTRAINT
==================================================

Do not build a giant speculative system before proving the core loop.

Implement in stages:

PHASE 1:

    AST extraction
    graph schema
    persistent storage
    basic CLI

PHASE 2:

    relationship resolution
    graph traversal
    impact analysis

PHASE 3:

    Claude Code skill
    MCP interface
    graph-first retrieval

PHASE 4:

    incremental updates
    Git integration
    architecture summaries

PHASE 5:

    semantic enrichment
    rationale/ADR connections
    visualization

At every phase, maintain a working system.

==================================================
27. ACCEPTANCE CRITERIA
==================================================

The feature is successful only if all of the following are true:

1. A repository can be indexed locally.

2. The index survives process/session termination.

3. A new Claude Code session can query the graph.

4. Claude can identify architectural relationships without first reading the entire repository.

5. Queries return compact relevant subgraphs.

6. AST-derived relationships are distinguishable from inferred relationships.

7. The graph updates incrementally after code changes.

8. Claude Code is instructed and/or automatically guided to use the graph before broad source exploration.

9. Raw grep/read remains available when graph retrieval is insufficient.

10. No source code needs to leave the local machine for basic indexing.

11. The implementation is language-adapter based.

12. FNDR's existing user-memory architecture remains isolated from codebase memory.

13. The system works outside the FNDR repository.

14. The system can eventually be installed into an arbitrary repository with one command.

The final desired developer experience should be approximately:

    cd any-repository

    fndr codegraph init

    fndr codegraph index

    fndr codegraph install-claude

Then:

    claude

and Claude should automatically have access to persistent structural knowledge of that repository.

==================================================
28. FIRST ACTION
==================================================

Do NOT immediately start writing code.

First inspect the existing FNDR architecture and determine:

    - existing CLI architecture
    - Rust crate/module structure
    - Tauri boundaries
    - existing persistence layers
    - existing LanceDB usage
    - existing embedding infrastructure
    - existing MCP/tool infrastructure
    - existing Claude/Codex agent integrations
    - existing configuration system
    - existing testing infrastructure

Then produce a concise implementation plan with:

    1. files/modules that should be created
    2. files/modules that should be modified
    3. graph schema
    4. indexing architecture
    5. retrieval architecture
    6. Claude Code integration architecture
    7. storage choice and justification
    8. incremental indexing strategy
    9. testing strategy
    10. phased implementation order

Do not ask me unnecessary questions.

Make reasonable engineering decisions from the existing repository.

After the plan is established, implement the feature incrementally, run tests after each major component, and continuously verify that the resulting architecture remains reusable for arbitrary codebases rather than becoming hard-coded to FNDR.
