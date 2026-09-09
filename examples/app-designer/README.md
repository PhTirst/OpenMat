# App Designer 示例

打开 `SignalApp.omui`，点击 **保存并运行**。默认示例通过原生 Rust 内核执行
`SignalApp.m` 类的私有 onStartup / onRun 方法，读取频率和幅度，计算并绘图。
`app` 是类实例，控件是它的成员属性。

布局保持浅灰、白色、蓝色主题：左侧组件库和对象树，中间画布 / XML / 代码，
右侧检查器，底部输出。23 种内置控件都有图标，派生控件沿用基类的图标与绘制方式。

## 文件与编辑

- `SignalApp.omui`：XML v2，保存应用类名、组件树、布局、属性覆盖和信号连接。
- `SignalApp.m`：继承 `openmat.ui.AppBase` 的应用类，保存状态与成员方法。
- `CounterButton.m`：继承 `openmat.ui.Button`，增加 Step、Count 属性和 Incremented 信号。
- `GainControl.m`：原有复合组件示例，保留私有内部控件及函数句柄用法。

选择 RunButton，检查器“事件”显示 `Clicked → app.onRun`。点击“编辑代码”
打开 SignalApp.m 并定位到方法。输入 `app.onAnalyze` 后编辑，会创建成员方法。
也可连接 `self.reset` 或 `OtherControl.reset`；其他对象的方法需要公开，
应用自己的处理方法可以私有。清空即断开连接。代码页可切换已打开的类文件。

保存只重新生成应用类中的 `% <OpenMat:components>` 标记区域：控件属性与
bindDesignerComponents 连接代码。业务方法写在区域外。可以直接使用
`app.Frequency.Value`、`app.Status.Text` 等真实对象属性。构造时设计器控件
尚未绑定，依赖控件的初始化放在 setup 或 onStartup；新应用默认连接 onStartup。

设计操作可撤销、重做。XML 修改需要“应用 XML”或保存后生效。保存检查文件
revision，外部冲突会报错；类与 XML 分别写入。原有 XML v1 应用仍以函数回调
方式打开和运行，不会自动转换。

## 自定义按钮与信号

1. 注册框输入 `CounterButton`，注册后从组件库添加它。
2. 检查器会读取类的公开属性，将 Step 改为 4。
3. 在“事件”页把 Incremented 连接到 `app.onCount`，点击编辑并填写：

   ```matlab
   function onCount(app, source, event)
       app.Status.Text = ['Count ' num2str(source.Count)];
   end
   ```

4. 保存并运行。连续点击按钮，Count 变为 4、8，应用通过 Incremented 信号
   更新状态文字。source 是按钮对象；Count 与 Step 是它的 M 类属性。

在 properties 增加公开属性，在 events 声明信号，用 addlistener 连接成员方法，
用 notify 发出信号。设计预览在独立会话执行 setup/update，预览属性与内部控件
不会写回 XML。外部修改组件类后，重新注册刷新元数据；保存应用类会刷新其检查器。
类文件需在当前目录或搜索路径，包类放在 `+package` 中。

## 可视化复合组件

打开 `ParameterApp.omui` 并保存运行：两个 `ParameterEditor` 实例分别拥有自己的
数值输入、滑块和状态。修改其中一个，另一个保持不变；三个按钮演示 M 代码动态
添加、删除实例，以及切换窗口宽度后调整网格布局。ValueChanged 连接到应用的
私有 onParameter 方法。

选择 First，点击 **编辑组件定义**，会打开 `ParameterEditor.omui`。内部三个控件
可按普通界面拖放和编辑；保存后更新 `ParameterEditor.m` 的生成区域。重新运行
ParameterApp，两个实例使用更新后的组件定义，各自的 Caption/Value 覆盖仍保留。

也可点击 **新建组件**，命名类、添加内部控件，保存同名 `.omui` / `.m` 文件。
在类的生成区域外声明公开属性和信号，用 protected update 同步内部控件。
应用中注册类名后即可拖入多个实例。复合组件的内部布局在定义中编辑，外部检查器
编辑实例属性和放置尺寸。已有运行实例需停止后重新运行，不做代码热替换。

运行时用 `parent.add(child)`、`parent.remove(child)` 和 `delete(child)` 维护组件树；
remove 只分离，delete 会清理子控件和监听器。用 `owner.listen(...)` 创建跟随 owner
销毁的监听器。setup 成功后只执行一次，refresh 递归调用 update；动态加入已初始化
容器的组件会自动初始化。M 中的 `obj.Layout` 与检查器布局同步到运行画布。

XML v3 保存复合组件定义，现有应用继续使用 XML v2。详见
`docs/rfcs/0008-reusable-designer-components.md`。

## 尺寸约束与批量布局

打开 `LayoutLab.omui` 并保存运行。`LayoutLab.m` 的启动和按钮成员函数计算信号、
绘图并写入底部日志；修改频率后点击“计算并绘图”可以验证真实 M 回调。

该示例使用 `260 1fr` 两列和 `auto 1fr 150` 三行：参数栏固定 260 像素，绘图区
填充剩余空间，日志栏固定 150 像素，标题随内容确定高度。停止运行后，将顶部
预览尺寸从 1100×720 改成 800×500，或拖动画布右下角手柄。参数栏会滚动，
绘图区域随窗口变化。预览尺寸会保存到窗口定义中，重新运行使用新尺寸。

检查器“布局”页可分别设置宽、高策略（默认、固定、随内容、填充剩余空间），
最小/最大尺寸、横向/纵向伸展，以及网格每行、每列尺寸。尺寸列表以空格分隔，
例如 `240 auto 1fr 2fr`；纯数字表示像素。留空恢复原有网格规则。
使用这些扩展字段时，`.omui` 自动带上 `layoutVersion="2"`，应用/组件的 XML
v2/v3 含义保持不变。需要同时更新前端和原生服务端。

编辑快捷操作：

- 在画布组件或对象树节点上右键，可复制、重命名、删除、上下移动、选择父容器，
  或包入布局容器。右键已有多选中的组件会保留多选，可批量排版和删除。
  Shift+F10 / 菜单键打开菜单，方向键选择、Enter 执行、Esc 关闭；F2 重命名。
- 拖动组件库与对象树之间的横向分隔条可调整两者高度，比例自动保存在当前浏览器。
  双击恢复默认比例；聚焦分隔条后用上下方向键调整，Home/End 调到可用范围两端。
- Shift / Ctrl / Cmd 单击切换多选；拖动容器空白处框选其直接子组件。
- 检查器显示共同属性和“多个值”；修改应用到所有所选组件。
- “对齐与尺寸”提供边缘/居中对齐、等间距和统一宽高；位置排版用于绝对布局。
- 绝对布局拖动按 8 px 吸附并显示参考线，Alt 临时关闭吸附。网格拖动行列，
  水平/垂直容器拖动顺序。方向键微调，Shift+方向键在绝对布局中移动 8 px。
- “包入容器”保留所选控件 ID，并创建水平、垂直、网格或滚动容器。
- Ctrl+D 复制，Delete 删除，Ctrl+Z 撤销，Ctrl+Shift+Z 重做；一次拖拽或批量
  操作只占一步历史，拖拽中按 Esc 取消。文本输入保留自身的编辑快捷键。

复合组件实例的内部布局仍在其定义文件中编辑。运行时检查器、热更新、自动
数据绑定和响应式断点留待后续阶段。规格见 `docs/rfcs/0009-designer-layout-editing.md`。

## 本地运行与验证

```powershell
cargo run -p openmat-server -- --listen 127.0.0.1:8768 --workspace-root examples/app-designer
```

另一个终端：

```powershell
$env:VITE_OPENMAT_WS_URL = 'ws://127.0.0.1:8768/kernel'
pnpm --dir apps/web dev
```

使用更新后的原生服务端；离线可编辑，执行 M 类需要内核，绘图需要 WebGPU。

```powershell
pnpm --dir apps/web test
pnpm --dir apps/web build
cargo build -p openmat-oex
cargo test -p openmat-bytecode -p openmat-runtime -p openmat-kernel
```

属性编辑器覆盖常用标量、列表、表格，不含完整属性验证、枚举/依赖属性编辑器、
任意对象图、通用重命名重构或 .mlapp 导入。详见
`docs/rfcs/0007-class-based-app-designer.md`。
