# Migration Status — greentic-session

- What changed: tightened TenantCtx enforcement (env/tenant/team must match; stored user must match when present) for create/update/lookups; `find_by_user` now fences strictly by caller context and cleans stale mappings; Redis backend is now opt-in (`--features redis`); version bumped to 0.4.3; added context fence tests.
- What broke / risks: callers that relied on changing team/user during updates will now receive `InvalidInput`; consumers expecting Redis by default must enable the `redis` feature explicitly; team-less callers cannot interact with team-bound sessions.
- Next repos to update: any runtime/CLI using greentic-session (e.g., greentic-runner, greentic-dev, greentic-deployer) must pass TenantCtx with the correct team and enable the `redis` feature in their manifests when needed.
