OpenMat 计算与绘图示例 / Numerical and plotting examples
=====================================================

第一次使用 / Getting started

1. 首次打开 OpenMat，左侧 Current Folder 就是示例文件夹。
   位置是系统“文档”中的 OpenMat\Examples\0.1.6。
2. 双击 START_HERE.m，在编辑器上方点击 Run，运行第一张函数曲线图。
3. 其他 .m 文件也可以单独打开并点击 Run，无需额外数据或工具箱。
   每个示例都可以重复运行；修改采样点数、参数或颜色，观察图形变化。
4. 三维 Figure 可拖动旋转、滚轮缩放；二维图形可在 Figure 工具栏导出 PNG。

On first launch, Current Folder already shows the examples. Open START_HERE.m
and click Run. Every .m file is standalone and uses only the bundled kernel.
If you previously opened another workspace, use Open Folder to select
Documents\OpenMat\Examples\0.1.6 (the Windows Documents folder may be redirected).
OPENMAT_WORKSPACE_ROOT overrides this default; no examples are copied into a
custom workspace. The installation's resources/examples folder also has them.

计算与二维绘图
  START_HERE.m              正弦、余弦和衰减振荡，含公式、图例与刻度
  fft_spectrum.m            50 Hz 和 120 Hz 混合信号，以及 FFT 单边幅度谱
  least_squares_fit.m       用矩阵左除完成二次拟合，同时展示拟合残差

数学图案
  golden_phyllotaxis.m      黄金角排列形成的彩色花序
  lissajous_light.m         不同频率形成的李萨如曲线
  wave_interference.m      两个波源叠加后的干涉场
  mathematical_mandala.m   多条相位不同的曲线形成的曼陀罗

三维绘图（建议逐个运行）
  chromatic_torus.m         彩色环面
  mobius_strip.m            莫比乌斯带
  harmonic_blossom.m        球面谐波花朵
  parametric_seashell.m     参数化海螺
  lorenz_attractor.m        洛伦兹系统的数值积分，可能需要几秒
  wave_planet.m             波纹星球

示例不包含 App Designer。每次启动只补充缺失的示例文件，不覆盖你的修改。
不同版本使用独立文件夹，升级不会改写旧版本示例。
这些是 OpenMat 项目自有示例，按项目 AGPL-3.0-only 许可证发布。
