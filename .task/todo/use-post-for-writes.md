# Switch write endpoints to POST

`/register`, `/heartbeat`, `/message/send` all use GET, which violates REST
semantics and makes responses cacheable by proxies/browsers unintentionally.

- `/register` → POST
- `/heartbeat` → POST (or PUT)
- `/message/send` → POST

Update the client CLI to match.
