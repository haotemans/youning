# 协议接收对象优先于引用作者

Status: accepted

2026-11-08：生产消息22289的at_me=1，引用群友原文并请求改写；Decision把云团的QQ编号误认为其他人，返回audience=other及ignore。引用内容正确入库，缺少Bot身份映射与明确指向契约导致误判。

在一致性快照中提供Bot身份别名与协议addressing，Decision和Chat共用，引用作者与当前请求对象分开。保留原始记录，引用不提供工具授权，多人真实@仍按语境判断。

Runtime仅对私聊或明确唯一@Bot的对象矛盾进行纠正：高把握、有效可见依据、非收尾的request/question/follow_up/correction若被误判other并ignore，则改为普通短文字回应，清除全部执行字段，不补调Decision。正常ignore、低把握、停止、收尾、无效依据和多人@不升级。此为ADR-0009及[ADR-0015](0015-heat-aware-social-participation.md)“ignore不强制回复”的窄范围例外；不能据此派工、生图或重放旧消息。

测试复现“引用他人＋只@Bot改写”的结构，同时覆盖多人@、他人对话、私聊、停止、收尾、普通ignore与伪造依据。mock验证协议契约及单次Decision调用，不代表真实模型正文质量或QQ交付。
