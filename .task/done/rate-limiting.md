# Rate limiting

Prevent inbox flooding and abuse via per-agent or per-IP request limits.

- Use a poem middleware layer (or `tower` compatible crate)
- Limit `/message/send` most aggressively (e.g. 60 req/min per sender)
- Return `429 Too Many Requests` with a `Retry-After` header

## Result

Builds cleanly. Here's what was added:

**`RateLimiter` middleware** (`server/src/main.rs`):

- **`/message/send` + `/message/broadcast`**: 60 req/min keyed by the `from` query param (falls back to IP if `from` is absent).
- **All other endpoints**: 300 req/min keyed by remote IP.
- **429 response** includes `Retry-After: 60` and a `{"error":"rate limit exceeded"}` JSON body.
- Sliding-window algorithm: each bucket holds a `VecDeque<Instant>` of request timestamps; old entries are pruned on every check.
- The `std::sync::Mutex` is dropped before the `.await` call to avoid holding it across yield points.
- Middleware is the outermost layer (applied last with `.with()`), so it runs before CORS and data injection.
