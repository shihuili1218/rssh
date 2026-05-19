# 当 SSH 数据流被结构化之后，AI、复制、折叠、审计这些能力自然长出来

## 一个被默认接受了 40 年的事实

终端是 byte 进、byte 出。

`\x1b[31mHello\x1b[0m\r\n` 进来，xterm 把红色 "Hello" 渲染出来。完事。
谁说的"红色"？哪一条命令的输出？输出从哪一行开始、到哪一行结束？

**终端不知道。它只知道字节。**

这个抽象选错了不止一次：

- 想复制上一条命令的输出 —— 滚轮往上滚，肉眼找起点
- 想把这段贴到 issue 里 —— 复制下来带一堆 ANSI 转义和软换行
- 想问 AI "这是什么错" —— 把整屏字节粘进网页，AI 看到的是字节、不是命令
- 想审计"昨天我跑了哪些命令" —— 没有"命令"这个对象，只有 scrollback 里的字节

40 年了，所有终端都默认接受了"我只是 byte 渲染器"。

## 最小的结构：每次 Enter 一刀

rssh 不发明 shell 集成，不在服务器上装 hook，不解析 prompt 正则。它做的是终端能做的最小动作：

**用户按 Enter，记一个 marker；下次按 Enter，前一个 marker 收尾，新一个 marker 开张。**

```ts
// src/lib/terminal/command-blocks.ts
term.onData(data => {
  if (term.buffer.active.type === "alternate") return;
  for (const ch of data) {
    if (ch === "\r") {
      closeCurrent();   // 给上一块标 end marker
      openNew();        // 给新一块标 start marker
    }
  }
});
```

整个文件 117 行。规则简单到没什么可写的。

但这里有个关键的选择：**marker 用的是 xterm.js 的 `IMarker`，不是行号、不是坐标、不是字节偏移。**

```ts
export interface CommandBlock {
  id: number;
  color: string;
  start: IMarker;
  end: IMarker | null;
}
```

`IMarker` 是 xterm.js 内部追踪行的对象。它有三个性质：

1. **跟着 scrollback 自动迁移** —— 终端往上滚 1000 行，marker 的 `line` 字段自动 -1000
2. **被修剪出 scrollback 时自动 dispose** —— 块从 tracker 里消失，不用手动清理
3. **resize 终端、reflow 软换行 —— marker 无感知**

也就是说：rssh 不维护行号、不监听 resize、不算坐标。**xterm.js 已经做了**，rssh 只是把"两个 marker 之间的一段"暴露成"块"这个抽象。

这就是全部的结构化代价 —— 117 行。

## 然后能力开始自己长出来

### 一、复制：从"字节段"到"纯文本"

复制不是 `getSelection().toString()`。块知道自己从哪行起、到哪行止 —— 就在 `start.line` 和 `end.line` 之间。

`src/lib/terminal/block-content.ts`（145 行）干两件事：
- 按行从 xterm buffer 里把 cell 数据取出来（cell 已经是字符 + 颜色 + 属性的结构化对象）
- 软换行按逻辑行合并、宽字符按 width=2 计算、ANSI 转义全丢掉

输出的纯文本贴到 GitHub issue 里直接可执行，不需要手动清理。

**这件事在"byte 流"抽象里是做不到的** —— 你不知道哪几行是"一个命令的输出"。在"块"抽象里，它就是一句 `block.start.line` 到 `block.end.line` 的切片。

### 二、复制为图片：同一份切片，换一个渲染器

`src/lib/terminal/block-to-image.ts`（375 行）。源数据完全一样 —— 块内每行的 cell —— 但渲染目标换成 canvas：

```
取 block.start..block.end 的所有 cell
  → 按当前终端字体在 canvas 上重画
  → 保留前景色、背景色、bold/italic
  → CJK 宽字符按 width=2
  → 软换行按逻辑行合并
→ PNG
```

贴 Slack 不会被压成模糊缩略图（不是截屏，是矢量重画），贴邮件不丢颜色，贴微信不漏字。

**这是同一把刀切出来的东西。** 复制为文本和复制为图片，共享 `block.start..block.end` 这个切片的定义 —— 渲染器不同而已。

### 三、折叠：把一段抽出去

这一刀最暴力。

折叠不是 CSS `display: none`。CSS 隐藏的行在滚动条里还占位、查找时还会被命中、复制时还会拷出来 —— **CSS 隐藏是骗 UI、不骗 buffer。**

rssh 干的是真的把这段 buffer 抽走：

```
// src/lib/terminal/folds.ts —— 304 行
fold:
  buffer.lines.splice(start, count)              // 把这段行抽出
  push 同样数量空行补足 ybase + rows            // 维持 xterm 长度不变量
  调整 ybase / ydisp / cursor.y                  // 渲染对齐

unfold:
  splice 把保存的行塞回原位
  pop 当初 push 的空行（cursor 有上下边界，未必能全 pop）
  cursor + ybase 同步
```

折叠完之后，xterm.js 自己都看不出来这段被动过 —— 滚动正常、查找正常、复制不漏拷被折叠的部分。

代价是依赖 xterm 的几个私有 API（`_core.buffer.lines.splice` / `addMarker` / `getBlankLine`），`package.json` 因此锁了 `@xterm/xterm@5.5.0`，升级前必须重跑 `folds.test.ts`。

**但这件事在"byte 流"抽象里同样不可能** —— 你不知道哪段该抽走。在"块"抽象里，"抽走"就是 `splice(block.start.line, block.end.line - block.start.line + 1)`。

### 四、AI：LLM 需要的不是字节，是命令单元

LLM 帮你看 CPU 跑满 —— 它跑一条 `top -bn1`，需要的不是"接下来 12KB 的 ANSI 字节"，是"这条命令的退出码 + 已脱敏的纯文本输出"。

rssh AI 的执行模型（`src-tauri/src/ai/session.rs`）是：

```
1. LLM 决定跑 cmd
2. rssh 生成 sentinel uuid
3. 把 `cmd; echo "<sentinel>:$?"` 粘到你的活动终端，自动回车
4. 前端监听 PTY 字节流，找 sentinel
5. 找到 → 提取 sentinel 之前的字节 → 切片 → 脱敏 → 截断 → 作为 tool_result 推回 LLM
```

注意第 5 步：**"sentinel 之前的字节"就是这条命令的块。** 这个切片的定义和"复制为文本"、"折叠"完全是同一个 —— 起点是命令回车那一刀、终点是 sentinel 出现的那一刀。

如果终端只是 byte 流，AI 想知道"这条命令输出到哪了"得猜：可能猜 prompt 正则、可能猜超时、可能猜空行。猜错就喂垃圾给 LLM、烧 token、误诊。

但 rssh 已经把"一条命令"建模成对象了。AI 工具只是这个对象的另一个消费者。

**LLM 不监听字节流，rssh 也不替它监听。** 命令在你的交互终端里完整可见，sentinel 是约定的标记位，仅此而已。这一条在 `session.rs:1-9` 的注释里写得很清楚：

> "命令在用户的交互终端里完整可见，没有任何后端注入或 byte 监控。"

### 五、审计：单位不是字节，不是时间，是"用户意图"

`src-tauri/src/ai/audit.rs` 里记的每一条都是块级别的：LLM 提议的命令、被拒的命令、被批准并执行的命令、它的退出码、它的（脱敏后）输出。

如果审计单位是字节，你打开审计日志看到的是 12KB ANSI；如果是时间，你看到的是"14:32:01 至 14:32:18 的输出"。两种都需要人脑再切一刀才能看懂。

块作为单位，审计直接可读：

```
[14:32:01] LLM proposed: jmap -histo:live <pid>
           side_effect: triggers Full GC, 100-300ms business pause
[14:32:03] User approved
[14:32:18] Exit 0, 8KB output
           [REDACTED:ip-10] × 3, [REDACTED:hex] × 1
```

**审计的可读性，是数据结构选对之后的副作用 —— 不是另写一套日志格式。**

## 共享的抽象：`block.start..block.end`

把这五件事铺开看，它们共享同一个抽象：

```
block.start: IMarker   ← Enter 时记下
block.end:   IMarker   ← 下次 Enter / 切 alternate buffer 时记下
```

- **复制为文本**：把这段 cell 取出来，丢 ANSI、拼软换行
- **复制为图片**：把这段 cell 取出来，丢给 canvas 渲染
- **折叠**：把这段 buffer splice 出去
- **AI 执行**：sentinel 之前那段就是 block，丢给 LLM
- **审计**：以块为最小记录单位

派生能力之间没有"特殊情况"，没有 if/else 补丁。一把刀切干净了，刀痕本身就是结构。

## 这不是产品规划，是数据结构选对了之后的副作用

很多 SSH 客户端的产品路线图长这样：

```
v1.5: 加 AI Chat 集成
v1.6: 加命令收藏功能
v1.7: 加输出折叠
v1.8: 加复制为图片
```

每一条都是独立的"功能"，每一条都得单独写代码、单独定义"一条命令的边界"、单独处理 ANSI、单独处理软换行、单独处理 resize ……

**这就是没选对数据结构的代价。** 同一个问题在每个功能里重新解一遍，每次解法略有差异，慢慢长出特殊情况、长出 bug、长出补丁。

rssh 的路径反过来：

```
先切一刀（IMarker pair per Enter）
  ↓
所有派生能力共享这把刀
  ↓
新功能不需要"再发明一次切刀的方式"
```

下一个能解锁的能力是什么？我不知道。但只要它需要"一条命令的输出"这个对象，它就已经长在那里了，等着被加个消费者。

---

**一句话**：选对一个数据结构，能力会自己长出来；选错了，每个能力都得从头解一遍。
