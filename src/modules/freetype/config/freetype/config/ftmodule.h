/*
 * ftmodule.h — FreeType 模块注册表（Tungsten 内核裸机配置）
 * 仅包含控制台渲染所需的模块：
 *   - autofit:  自动提示（提高小字号可读性）
 *   - truetype: TrueType 字形解码器
 *   - sfnt:     SFNT 容器解析（OpenType / TrueType 字体）
 *   - smooth:   抗锯齿光栅化
 *   - cff:      CFF 轮廓解码器（OpenType CFF 字体如思源黑体、Noto CJK）
 *   - psaux:    PostScript 辅助模块（CFF 驱动依赖）
 *   - psnames:  PostScript 字形名称模块（CFF 驱动依赖）
 * Copyright (C) 2026 Nexsteaduser. All rights reserved.
 */

FT_USE_MODULE( FT_Module_Class, autofit_module_class )
FT_USE_MODULE( FT_Module_Class, tt_driver_class )
FT_USE_MODULE( FT_Module_Class, sfnt_module_class )
FT_USE_MODULE( FT_Module_Class, ft_smooth_renderer_class )
FT_USE_MODULE( FT_Module_Class, cff_driver_class )
FT_USE_MODULE( FT_Module_Class, psaux_module_class )
FT_USE_MODULE( FT_Module_Class, psnames_module_class )
