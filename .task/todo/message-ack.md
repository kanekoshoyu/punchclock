# Message acknowledgment

Add delivery confirmation so senders can know a message was received.

- `POST /message/ack?agent_id=&message_id=`
- Messages need a stable `id` field (UUID)
- Track acked vs unacked; expose ack status to sender via a status endpoint
- Optional: redeliver unacked messages after a timeout
