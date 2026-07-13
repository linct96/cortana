# 常见开发问题

## macOS 中输入框聚焦后需要点击两次按钮

### 现象

在 Tauri 的 macOS 窗口中，弹窗内的 `input` 或 `textarea` 聚焦后，第一次点击 Base UI 的按钮或切换项只让输入框失焦，第二次点击才触发操作。

### 原因

正常的 WebKit 事件顺序是 `mousedown -> blur -> focusout -> mouseup -> click`。当前 WKWebView 与 Base UI 组合表现为输入框失焦后，同一次操作没有继续触发组件的 `click` 或 Toggle 状态处理。

这不是 Tauri 的 `acceptFirstMouse` 场景。`acceptFirstMouse` 只控制点击未激活窗口时是否同时把点击传给 WebView；此问题发生时输入框已经聚焦，窗口本身处于激活状态。

### 处理

只在弹窗内不应抢占输入焦点的按钮和切换项上取消 `mousedown` 的默认焦点转移，业务仍由原有 `click`、`onValueChange` 和键盘事件处理：

```tsx
<Button onMouseDown={(event) => event.preventDefault()} onClick={handleAction}>
  操作
</Button>
```

弹窗关闭按钮使用 `DialogClose`，由 Base UI 统一处理关闭与焦点恢复：

```tsx
<DialogClose
  render={<Button onMouseDown={(event) => event.preventDefault()} />}
>
  取消
</DialogClose>
```

不要把业务逻辑改到 `mousedown` 中，也不要全局阻止 `blur`。前者会破坏键盘操作和按下后移出按钮取消点击的语义，后者会让正常的表单焦点切换失效。

### 参考

- [WebKit 中按钮点击时的焦点事件顺序](https://www2.webkit.org/show_bug.cgi?id=229895)
- [Tauri `accept_first_mouse`](https://docs.rs/tauri/latest/src/tauri/webview/webview_window.rs.html#897-901)
- [`mousedown` 阶段阻止输入框失焦的社区讨论](https://stackoverflow.com/questions/7621711/how-to-prevent-blur-running-when-clicking-a-link-in-jquery)
