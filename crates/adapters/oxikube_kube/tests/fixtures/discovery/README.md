# Discovery fixtures

Small hand-built discovery documents in the two wire shapes, describing the same cluster
(core, `apps`, `autoscaling` v2 + v1, `authentication.k8s.io`, and the E01-S09 sample CRD group
`test.oxikube.dev`). Unit tests assert both shapes convert to the same registry.

- `aggregated.json`: `{"api": <APIGroupDiscoveryList for /api>, "apis": <... for /apis>}`
  (`apidiscovery.k8s.io/v2`; versions in preference order, the first is preferred).
- `legacy.json`: request path -> response body (`/api`, `/apis`, `/api/v1`, `/apis/<group>/<version>`).

Deliberate edge cases: subresources (`pods/log`, `deployments/scale`), kinds without `get`/`list`
(`Binding`, `TokenReview`), a group with two versions (`autoscaling`), cluster-scoped kinds,
short names and categories.
