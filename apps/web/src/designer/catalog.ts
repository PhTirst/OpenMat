/** The designer and preview use the same control definitions. Property values are
 * instance overrides; the XML never contains DOM nodes or runtime handles. */
export type UiValue =
    | string
    | number
    | boolean
    | string[]
    | (string | number | boolean)[][];
export interface PropertySpec {
    name: string;
    label: string;
    type:
        | "text"
        | "number"
        | "boolean"
        | "color"
        | "items"
        | "table"
        | "choice";
    default: UiValue;
    choices?: string[];
    min?: number;
    max?: number;
    readonly?: boolean;
    /** Native storage type discovered from the class, never copied into XML. */
    valueClass?: string;
}
export interface ComponentSpec {
    type: string;
    label: string;
    group: string;
    icon: string;
    container: boolean;
    properties: PropertySpec[];
    events: string[];
    className?: string;
    renderType?: string;
    methods?: MethodSpec[];
    composite?: boolean;
    defaultLayout?: Partial<import("./model").UiLayout>;
}
export interface MethodSpec {
    name: string;
    access: "public" | "protected" | "private";
    declaringClass: string;
}
export function isUiValue(value: unknown): value is UiValue {
    const scalar = (v: unknown) =>
        typeof v === "string" ||
        typeof v === "boolean" ||
        (typeof v === "number" && Number.isFinite(v));
    return (
        scalar(value) ||
        (Array.isArray(value) &&
            value.length <= 4096 &&
            (value.every((v) => typeof v === "string") ||
                value.every(
                    (row) =>
                        Array.isArray(row) &&
                        row.length <= 4096 &&
                        row.every(scalar),
                )))
    );
}
const text = (name: string, label: string, value = ""): PropertySpec => ({
    name,
    label,
    type: "text",
    default: value,
});
const number = (
    name: string,
    label: string,
    value: number,
    min?: number,
    max?: number,
): PropertySpec => ({
    name,
    label,
    type: "number",
    default: value,
    ...(min === undefined ? {} : { min }),
    ...(max === undefined ? {} : { max }),
});
const bool = (name: string, label: string, value: boolean): PropertySpec => ({
    name,
    label,
    type: "boolean",
    default: value,
});
const choice = (
    name: string,
    label: string,
    value: string,
    choices: string[],
): PropertySpec => ({ name, label, type: "choice", default: value, choices });
const items: PropertySpec = {
    name: "Items",
    label: "选项",
    type: "items",
    default: ["选项 1", "选项 2", "选项 3"],
};
const common = [
    bool("Visible", "可见", true),
    bool("Enable", "启用", true),
    text("Tooltip", "提示"),
];
const ranges = [
    number("Min", "最小值", 0),
    number("Max", "最大值", 100),
    number("Step", "步长", 1, 0.000001),
];
function spec(
    type: string,
    label: string,
    group: string,
    properties: PropertySpec[] = [],
    events: string[] = [],
    container = false,
): ComponentSpec {
    return {
        type,
        label,
        group,
        icon: type,
        properties: [...properties, ...common],
        events,
        container,
    };
}
export const BUILTIN_COMPONENTS: ComponentSpec[] = [
    spec(
        "Window",
        "窗口",
        "容器与布局",
        [
            text("Title", "标题", "我的应用"),
            number("Width", "宽度", 840, 240, 4096),
            number("Height", "高度", 560, 160, 4096),
        ],
        ["Startup"],
        true,
    ),
    spec(
        "Panel",
        "面板",
        "容器与布局",
        [text("Title", "标题", "面板")],
        [],
        true,
    ),
    spec(
        "TabGroup",
        "选项卡组",
        "容器与布局",
        [number("SelectedIndex", "当前页", 1, 1)],
        ["SelectionChanged"],
        true,
    ),
    spec(
        "Tab",
        "选项卡",
        "容器与布局",
        [text("Title", "标题", "页面")],
        [],
        true,
    ),
    spec("GridLayout", "网格布局", "容器与布局", [], [], true),
    spec("RowLayout", "水平布局", "容器与布局", [], [], true),
    spec("ColumnLayout", "垂直布局", "容器与布局", [], [], true),
    spec(
        "ScrollPanel",
        "滚动容器",
        "容器与布局",
        [
            choice("ScrollDirection", "滚动方向", "vertical", [
                "vertical",
                "horizontal",
                "both",
            ]),
        ],
        [],
        true,
    ),
    spec(
        "SplitPane",
        "分隔面板",
        "容器与布局",
        [
            choice("Orientation", "方向", "horizontal", [
                "horizontal",
                "vertical",
            ]),
        ],
        [],
        true,
    ),
    spec("Label", "标签", "文字与操作", [
        text("Text", "文本", "标签"),
        number("FontSize", "字号", 13, 8, 96),
    ]),
    spec(
        "Button",
        "按钮",
        "文字与操作",
        [
            text("Text", "文本", "按钮"),
            choice("Variant", "外观", "default", ["default", "primary"]),
        ],
        ["Clicked"],
    ),
    spec(
        "TextField",
        "文本输入",
        "输入与选择",
        [text("Value", "值"), text("Placeholder", "占位文本", "请输入…")],
        ["ValueChanged"],
    ),
    spec(
        "TextArea",
        "多行文本",
        "输入与选择",
        [text("Value", "值"), text("Placeholder", "占位文本", "输入文本…")],
        ["ValueChanged"],
    ),
    spec(
        "NumericField",
        "数值输入",
        "输入与选择",
        [number("Value", "值", 1), ...ranges],
        ["ValueChanged"],
    ),
    spec(
        "CheckBox",
        "复选框",
        "输入与选择",
        [text("Text", "文本", "启用选项"), bool("Value", "选中", false)],
        ["ValueChanged"],
    ),
    spec(
        "RadioGroup",
        "单选组",
        "输入与选择",
        [items, text("Value", "值", "选项 1")],
        ["ValueChanged"],
    ),
    spec(
        "DropDown",
        "下拉选择",
        "输入与选择",
        [items, text("Value", "值", "选项 1")],
        ["ValueChanged"],
    ),
    spec(
        "Slider",
        "滑块",
        "输入与选择",
        [number("Value", "值", 50), ...ranges],
        ["ValueChanged"],
    ),
    spec(
        "Table",
        "数据表格",
        "数据与显示",
        [
            {
                name: "Columns",
                label: "列标题",
                type: "items",
                default: ["参数", "数值"],
            },
            {
                name: "Data",
                label: "数据",
                type: "table",
                default: [
                    ["频率", 1],
                    ["幅度", 1],
                ],
            },
            bool("Editable", "允许编辑", false),
        ],
        ["CellEdited", "SelectionChanged"],
    ),
    spec("Image", "图像", "数据与显示", [
        text("Source", "图片 URL"),
        text("Alt", "替代文本", "图像"),
        choice("Fit", "适应方式", "contain", ["contain", "cover"]),
    ]),
    spec("PlotView", "绘图视图", "数据与显示", [
        text("Title", "标题", "绘图"),
        number("FigureIndex", "绘图序号", 1, 1),
    ]),
    spec("ProgressBar", "进度条", "状态与反馈", [
        number("Value", "进度 (%)", 40, 0, 100),
    ]),
    spec("StatusLamp", "状态灯", "状态与反馈", [
        text("Text", "文本", "就绪"),
        { name: "Color", label: "颜色", type: "color", default: "#247951" },
    ]),
    spec(
        "ComponentContainer",
        "自定义容器",
        "自定义组件",
        [text("Title", "标题", "自定义组件")],
        [],
        true,
    ),
];
export const BUILTIN_CATALOG: Readonly<Record<string, ComponentSpec>> =
    Object.fromEntries(
        BUILTIN_COMPONENTS.map((component) => [component.type, component]),
    );
export function propertyValue(
    node: { properties: Record<string, UiValue>; type: string },
    name: string,
    catalog = BUILTIN_CATALOG,
): UiValue {
    return (
        node.properties[name] ??
        catalog[node.type]?.properties.find((p) => p.name === name)?.default ??
        ""
    );
}
