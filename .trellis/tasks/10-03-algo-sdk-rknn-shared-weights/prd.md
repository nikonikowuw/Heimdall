# SDK placement 注入与 RKNN 共享权重运行时

## Goal

升级 InitContext 为 Builder，剥离 RKNN_CORE_MASK 进程覆盖，接入真实 rknn_dup_context 权重复用 provider，移除人脸与 YOLO 的内部全局串行 Actor

## Requirements

- TBD

## Acceptance Criteria

- [ ] TBD

## Notes

- Keep `prd.md` focused on requirements, constraints, and acceptance criteria.
- Lightweight tasks can remain PRD-only.
- For complex tasks, add `design.md` for technical design and `implement.md` for execution planning before `task.py start`.
