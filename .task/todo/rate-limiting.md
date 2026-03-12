# Rate limiting

Prevent inbox flooding and abuse via per-agent or per-IP request limits.

- Use a poem middleware layer (or `tower` compatible crate)
- Limit `/message/send` most aggressively (e.g. 60 req/min per sender)
- Return `429 Too Many Requests` with a `Retry-After` header
