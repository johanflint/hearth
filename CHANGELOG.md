# Changelog

## v0.2.0 (2026-09-19)

### Features

- Provide an installer script ([#40](https://github.com/johanflint/hearth/pull/40))

### Bug Fixes

- Allow setting readonly values from observers but not from flows ([#42](https://github.com/johanflint/hearth/pull/42))

## v0.1.0 (2026-09-17)

### Features

- Include default config ([#39](https://github.com/johanflint/hearth/pull/39))
- Implement metrics ([#21](https://github.com/johanflint/hearth/pull/21))
- Support polar days and nights ([#18](https://github.com/johanflint/hearth/pull/18))
- Deterministic conflict resolution ([#17](https://github.com/johanflint/hearth/pull/17))
- Support conditional nodes ([#16](https://github.com/johanflint/hearth/pull/16))
- Schedule sunrise/sunset events ([#15](https://github.com/johanflint/hearth/pull/15))
- Support temporal expressions ([#14](https://github.com/johanflint/hearth/pull/14))
- Support sleep node in flows ([#13](https://github.com/johanflint/hearth/pull/13))
- Scheduled events ([#12](https://github.com/johanflint/hearth/pull/12))
- Support and, or and not expressions ([#10](https://github.com/johanflint/hearth/pull/10))
- Set color property value ([#9](https://github.com/johanflint/hearth/pull/9))
- Introduce expressions ([#8](https://github.com/johanflint/hearth/pull/8))
- Set number property ([#7](https://github.com/johanflint/hearth/pull/7))
- Introduce commands ([#6](https://github.com/johanflint/hearth/pull/6))
- Update properties ([#5](https://github.com/johanflint/hearth/pull/5))
- Map Hue light properties ([#4](https://github.com/johanflint/hearth/pull/4))
- Flow engine ([#3](https://github.com/johanflint/hearth/pull/3))
- Hue observer ([#2](https://github.com/johanflint/hearth/pull/2))
- Listen to SSE stream ([#1](https://github.com/johanflint/hearth/pull/1))

### Bug Fixes

- Fix typo in Grafana SSE connection attempts metric ([#33](https://github.com/johanflint/hearth/pull/33))
- Fix panics by returning an error ([#32](https://github.com/johanflint/hearth/pull/32))
- Fix `Number` subtraction for mixed positive and negative integers ([#31](https://github.com/johanflint/hearth/pull/31))
- Prevent Hue light control panics ([#29](https://github.com/johanflint/hearth/pull/29))
- Fix client request timeout ([#28](https://github.com/johanflint/hearth/pull/28))
- Reject duplicate flow ids ([#27](https://github.com/johanflint/hearth/pull/27))
- Add a timeout to client requests ([#26](https://github.com/johanflint/hearth/pull/26))
- Fix `Number` addition for mixed positive and negative integers ([#25](https://github.com/johanflint/hearth/pull/25))
- Fix store not persisting property changes ([#22](https://github.com/johanflint/hearth/pull/22))
- Harden sse listener recovery ([#20](https://github.com/johanflint/hearth/pull/20))
- Prevent Hue color-temperature mapping panic ([#19](https://github.com/johanflint/hearth/pull/19))
- Fix SSE connection not always reconnecting ([#11](https://github.com/johanflint/hearth/pull/11))

### Miscellaneous Chores

- Observability through Prometheus and Grafana ([#24](https://github.com/johanflint/hearth/pull/24))

### Tests

- Introduce DeviceBuilder for streamlined test setup ([#23](https://github.com/johanflint/hearth/pull/23))

### Build System

- Create release workflows ([#37](https://github.com/johanflint/hearth/pull/37))

### Continuous Integration

- Improve ci pipeline ([#36](https://github.com/johanflint/hearth/pull/36))
- Fix CI workflow redundancy ([#34](https://github.com/johanflint/hearth/pull/34))
