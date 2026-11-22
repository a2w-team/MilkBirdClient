# 校园公告查询 · 后台 HTTP v1

托盘向 Hertz 发送 `POST /api/v1/announcement-lookups`，正文为 UTF-8 JSON，最大 8 KiB。请求只包含用户已确认的两个字段：

```json
{"campus":"东校区","topic":"期末考试"}
```

服务端去掉字段两侧的空白，每字段最多 80 个 Unicode 字符。输入为非法 JSON、字段类型错误、包含未知字段或长度超限时返回 HTTP 400 和 `{"error":"invalid_query"}`。服务端会拒绝超出请求体上限的请求。

HTTP 200 对应三种结果：

| status | 附加字段 | 含义 |
| --- | --- | --- |
| `matched` | `announcement` | 只返回一条有效公告。 |
| `needs_clarification` | `missingFields` | 包含缺少的 `campus`、`topic`。 |
| `no_match` | 无 | 没有匹配的有效公告，托盘展示实际结果。 |

`announcement` 包含 `id`、`campus`、`topic`、`title`、`summary`、`source`、`sourceUrl`、`publishedAt` 和 `validUntil`。来源地址为 HTTPS，时间使用 RFC 3339。校区与主题按规范化后的精确值匹配。只取当前已发布且尚未过期的公告，按发布时间最新优先，同一时间按公告 ID 升序选一条。

该查询只读。首条联调使用本机 Hertz 和显式加载的合成公告，部署到网络之前需接入俱乐部设备鉴权、信息可见范围和正式数据源。
