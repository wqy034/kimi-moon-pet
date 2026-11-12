# 配置多组 API 连接

在「交流设置」中配置聊天、生活回应和后台记忆整理使用的 API。最多可保存 16 组连接。

## 使用

1. 点击「添加连接」，选服务商预设或自定义接口，再填写 Key 与模型 ID。自定义连接还需填写 API 地址。同一家服务商可保存多组连接。
2. 点击「查询模型」会先保存未保存的连接参数，再向该连接的 `GET /models` 查询目录；不生成聊天回复。目录可搜索，点击模型即保存选用结果。模型ID也能手填。目录含义以服务商为准，列出不代表适合文字聊天。
3. 单击连接行展开详情，“设为主用”决定每次请求的起点；勾选「可作备选」，用 ↑↓ 调整备选顺序，再打开「失败时自动使用备用」。自动切换默认关闭。
4. 设置页显示 Key 的首尾各四位，长度不超过八位时全部遮罩。

Base URL可以填公共HTTPS接口，或本机HTTP接口（localhost/127.0.0.1/::1）。粘贴末尾 `/chat/completions` 或 `/models` 会规范化为Base URL；不自动补 `/v1`，保留不同平台所需的路径。禁止URL账号、密码、查询参数和片段。接口地址变更后必须重新填Key；加密凭据绑定准确的连接ID和规范化地址，防止旧Key随修改后的地址发送。

未保存的新Key只提交给Rust；保存/收起/关闭后清空输入框。查询失败或返回格式不兼容时显示原因，可继续手填模型；不会偷偷换另一个平台查询。查询目录缓存和模型选择在重启后保留。点击模型保存时也会按原配置规则撤销当前生成，请在结束回复后调整。

## 预设与协议

支持 **OpenAI Chat Completions** 兼容接口。预设是便利填写入口，不保证账号开通权限、目录完整或每个模型都支持当前文字协议。模型 ID 可以查询或手动填写。

| 预设 | Base URL | 官方资料 |
| --- | --- | --- |
| DeepSeek 官方 | `https://api.deepseek.com` | [API文档](https://api-docs.deepseek.com/) |
| 硅基流动 | `https://api.siliconflow.cn/v1` | [Chat接口](https://docs.siliconflow.cn/docs/api/chat-completions-post) |
| OpenAI | `https://api.openai.com/v1` | [Chat Completions](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)、[模型列表](https://developers.openai.com/api/reference/resources/models/methods/list) |
| Google Gemini兼容接口 | `https://generativelanguage.googleapis.com/v1beta/openai` | [兼容协议](https://ai.google.dev/gemini-api/docs/openai) |
| 阿里云百炼 / 通义千问 | `https://dashscope.aliyuncs.com/compatible-mode/v1` | [兼容协议与地域地址](https://help.aliyun.com/zh/model-studio/compatibility-of-openai-with-dashscope) |
| 月之暗面 Kimi | `https://api.moonshot.cn/v1` | [官方平台](https://platform.moonshot.cn/docs/api-reference) |
| 智谱 GLM | `https://open.bigmodel.cn/api/paas/v4` | [兼容协议](https://docs.bigmodel.cn/cn/guide/develop/openai/introduction) |
| OpenRouter | `https://openrouter.ai/api/v1` | [API参考](https://openrouter.ai/docs/api-reference/overview) |

百炼的新业务空间域名和不同地域地址可以手填，各地域Key不可混用。其他聚合网关、兼容服务可用自定义；本机免鉴权接口可填一个占位Key（如 `local`）。不支持直接填原生Anthropic Messages、OpenAI Responses或Gemini原生协议的URL。

请求参数按兼容类型映射，避免把硅基流动专属参数发给所有平台。OpenAI使用 `max_completion_tokens`，其他类型使用 `max_tokens`。DeepSeek/Kimi/GLM发送关闭思考参数，硅基流动/百炼发送 `enable_thinking=false`，Gemini采用低思考档；模型强制思考时不保证能彻底关闭。通用兼容类型只发送基础参数，记忆整理依靠JSON提示词与Rust校验，不强加JSON模式参数。聊天、生活回应和记忆整理的输出上限分别为 1024、128 和 1200 token；某些强制思考模型可能不适合这个短输出预算。

## 自动切换

开关作用于文字聊天、生活API回应与后台记忆整理。每次从主连接开始，随后按列表顺序尝试已勾选的备选；同一轮内每组连接最多一次，不永久改写主连接。每组只加载自己的地址、Key和模型，禁止把一个平台的Key带到另一个平台。

可切换条件：连接失败、响应/读流等待30秒超时、401～404、408、409、425、429、5xx；兼容SSE内明确的HTTP类错误码同样处理。正常完成但空回复也可尝试备选。每条聊天或每轮整理共享120秒总时限，不给每家额外120秒。

普通聊天一旦收到正文便不自动重发，失败保留部分内容。输出达到长度上限、内容过滤、工具调用、格式错误等不作为跨平台重试条件。停止、清空和配置修改先撤销代次/取消任务，旧流与旧模型目录不得写入或展示。

记忆整理不向用户展示流，只有通过JSON及来源校验后才写入记忆；网络或服务商错误可以尝试备选，模型格式/证据校验错误不跨商补写。整个整理轮次失败后仍沿用最多再自动重试一轮的规则。整理任务保留原主连接归属和实际返回连接，重启可恢复；改主连接后可切回继续旧待办，或明确手动“按当前服务商重试待办”。

打开自动切换意味着同一份聊天上下文、状态或记忆来源可能发给多家已启用的连接，并可能产生多次费用。设置页已显示此说明。失败/取消不保证免计费；没有usage时保持缺失，不伪造0。生活事件冷却仍按成功事件计数，不把每个失败HTTP尝试当成新的生活事件。

## 本地存储与升级

交流数据库当前为 schema6。连接地址、模型目录、主用连接和开关保存在数据库中，Key 由 Windows DPAPI 单独加密，绑定连接 ID 与接口地址。旧版内置 DeepSeek 和硅基流动连接有兼容读取路径。较旧的程序无法打开较新的数据库，升级前请备份交流数据。详见[数据库迁移与备份](记忆底层重整-实现与验证-20261103.md#迁移与备份)。

原始交流的每条回复可展开“使用的连接与尝试”，查看连接名、模型、状态、错误及服务商实际usage。成功回复记录实际使用的连接；失败尝试和部分正文不进入正常短期上下文或长期记忆。正在使用/切换的连接在设置页显示，输入气泡不接收连接列表和Key尾号。

清空全部交流与记忆也清空尝试记录，保留已保存连接、目录、开关和凭据。移除连接会清理对应加密Key，不删除历史或记忆。

## 验证连接

使用测试连接和虚构内容核对以下行为。真实服务商请求可能产生费用。

- 查询并选用模型。若服务商不提供模型目录，手填模型 ID 后验证连接。
- 配置两组测试连接，分别检查主用成功、备用接手和关闭自动切换时的行为。
- 查看「原始记录」中的连接尝试，核对实际连接、状态和服务商返回的用量。
- 更改接口地址后重新填写 Key。更换连接、停止回复或清空记录后，检查旧请求的内容是否仍会出现。
