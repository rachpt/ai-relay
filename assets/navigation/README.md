# 原生 Tab 导航图标

四张 SVG 是本项目新绘制的原创线性图标（概览仪表盘、统计柱图、日志文档、设置齿轮），不是字体字符，也未引入第三方图标包、字体或其许可证。这些 SVG 及其生成遮罩与项目一样采用根目录 [MIT 许可证](../../LICENSE)。

## 离线生成与验证

在 Windows PowerShell 5.1 / .NET Framework 环境运行，无需安装模块，无网络请求，不启动 GUI：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/generate-navigation-icons.ps1 -SelfTest
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/generate-navigation-icons.ps1 -ValidateOnly -SelfTest
```

路径相对脚本目录解析，不依赖当前工作目录。第一条重建并覆盖本目录四个 `.mask`；第二条仅重绘到内存并逐字节比较现有文件，检查尺寸、透明边缘、实心/抗锯齿像素，以及拒绝危险/不支持 XML 的回归用例。相同 Windows System.Drawing 实现下输出确定；跨 Windows/GDI+ 版本不承诺像素完全相同，差异应人工审查后再提交。正常 Cargo 构建不调用此脚本，必须将生成的 `.mask` 一并纳入版本控制。

### 明确支持的 SVG 子集

这不是通用 SVG 引擎，不支持任意 SVG：

- 最大 16 KiB；根元素为标准 SVG 命名空间的 `svg`，固定 `viewBox="0 0 24 24"`、`fill="none"`、`stroke="#000000"`、`stroke-width="1.65"`、圆端点和圆连接。
- 根元素下仅允许纯文字 `title` / `desc`、无圆角 `rect(x,y,width,height)`、`circle(cx,cy,r)`、`path(d)`，最多 128 个子节点；可有 XML 注释和空白。
- 路径仅允许绝对 `M x y`、`L x y`、`H x`、`V y`、`Z`。每段必须显式写命令；`Z` 后要开始新路径时必须写 `M`。不支持隐式重复、相对坐标、曲线、弧线或科学计数法。
- 坐标为 0–24 的非负十进制数；矩形/圆尺寸必须为正且在画布内；栅格结果四边必须完全透明。
- 未列出的标签/属性、分组、变换、样式、嵌入图片、脚本、链接、外部资源、处理指令、DTD 和实体均拒绝，XML resolver 禁用。所有形状只描边不填充。

System.Drawing 的公开 `GraphicsPath` / `DrawPath` API 读取上述 SVG 几何，4 倍超采样后对每个 4×4 区域的 alpha 求均值，不另写一套重复的图标形状。圆由公开椭圆路径 API 栅格化，SVG 源不含弧线路径命令。

### 二进制格式

每个 `.mask` 为 30,613 字节：ASCII `NAV1` 后顺序存放 18、27、36、48、72、144 像素六帧；每帧为 little-endian `u16` 边长、`u32` 数据长度和按行从上到下排列的 `size²` 个 8 位 alpha coverage。无 RGB、压缩、路径、外部引用或运行时解析器。Rust 校验所有帧头、长度及文件尾，异常时只显示文本。

## 运行时与部署

- `src/ui/tabs.rs` 通过子模块 `tabs/icons.rs` 的 `include_bytes!` 嵌入四份遮罩，不读取磁盘，不需要父模块接线、Cargo 依赖或 `build.rs` 修改。
- 图标为 18 DIP；常用 DPI 96/144/192/256/384/768 直接使用对应像素帧，其他 DPI 从最近的较大帧一次性双线性重采样；DPI 限制在 96–768，避免失控分配。
- `COLORREF` 转换为预乘 BGRA DIB；当前 DPI 下每个图标最多缓存两个颜色（正常使用为文字色/选中色），缓存保留最多 8 个 memory DC + 8 个 bitmap（原始 DIB 像素合计最多 663,552 字节）；替换时先成功创建新对象再释放旧对象，瞬时最多 9 对对象、746,496 字节 DIB 像素。主题变更按两槽轮换替换；DPI 变化清空旧缓存；退出时 RAII 还原 stock bitmap 并释放对象。
- 每次绘制只作等尺寸 `AlphaBlend`；像素循环只发生在有界缓存缺失时，无逐像素 GDI 调用。此公开 API 直接链接 Windows 10/11 自带的 `msimg32.dll`；无需附带 DLL、System.Drawing、PowerShell、SVG 引擎或语言运行时。
- 标签保持原生纯文字“概览 / 统计分析 / 实时日志 / 设置”。在原生 112 DIP item 内测量文字后居中排列图标与 6 DIP 间隔；空间不足、GDI 分配/绘制失败时只显示完整文字。高对比度完全绕过自绘，使用原生文本 Tab。
- 不改 Tab 命中区域、选择消息、键盘处理、焦点框或可访问名称。高对比度/读屏/物理键盘/多屏 DPI 仍需 Windows 实机验收。

`tabs/icons.rs` 包含帧/损坏输入、透明度/着色/DPI、缓存复用与 GDI 对象释放测试；本任务按并行分工只运行离线 PowerShell 验证，Rust 测试及全项目构建由父任务统一执行。GDI 资源测试检查独立内存 DC 所有权，不代替完整 GUI 长稳/泄漏验收。
