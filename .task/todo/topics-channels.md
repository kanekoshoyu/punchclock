# Topics / channels

Allow agents to subscribe to named topics. Messages sent to a topic fan out to
all subscribers, enabling pub-sub without point-to-point wiring.

- `POST /topic/subscribe?agent_id=&topic=`
- `POST /topic/unsubscribe?agent_id=&topic=`
- `POST /message/publish?topic=&from=&body=` — delivers to all subscribers' inboxes
