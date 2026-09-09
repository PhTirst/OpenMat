# OpenMat conformance cases

These are clean-room, OpenMat-authored programs for comparing observable
language behavior with MATLAB R2022b. They do not contain MATLAB source,
tests, documentation, or diagnostic prose.

## Layout

- `schema/case.schema.json` defines the project-owned case manifest format.
- `schema/observation.schema.json` defines normalized oracle/runner output.
- `cases/manifests/*.json` contains one manifest per case.
- `cases/programs/*.m` contains scripts that assign `openmat_result`.
- `cases/support/*.m` contains OpenMat-authored helper functions and classes.
- `reference/matlab-r2022b/*.json` is generated black-box reference output.

Every manifest has a stable `id`, a relative `source`, feature `tags`, and an
`expected` observation. An expected successful value specifies its `class`,
`size`, `ndims`, `numel`, normalized `kind`, and payload. Floating values are
stored as locale-independent strings so that `NaN`, infinities, signed zero,
and full precision survive JSON. A case may add absolute and relative
tolerances. Expected errors contain only an OpenMat-owned category.

The script contract is deliberately small: execute the source in a fresh
workspace and read `openmat_result` if it succeeds. A source that is expected
to fail may throw before assigning it. Support files are placed on the path by
the harness.

## Runner-neutral comparison interface

An OpenMat conformance runner should accept the same manifests and produce one
JSON file per case conforming to `schema/observation.schema.json`. Comparison
is then independent of the execution engine:

1. Match `outcome`.
2. For success, match `class`, `size`, `ndims`, `numel`, and `kind` exactly.
3. Compare numeric `real`/`imag` component strings exactly unless a manifest
   supplies `tolerance`; then parse finite strings and apply
   `abs(a-e) <= absolute + relative * max(abs(a), abs(e))` element-wise.
4. Compare logical, character-code, string, integer, and missing-value payloads
   exactly and in column-major linear order.
5. For errors, match only `error.category`; never compare proprietary messages.

## Corpus index

The corpus contains 295 manifests and 295 checked-in MATLAB R2022b reference
observations. Every identifier below has exactly one manifest, one program, and
one reference observation.

| Area | Count | Case identifiers |
| --- | ---: | --- |
| Foundations and operators | 12 | `scalar_shape`, `matrix_shape`, `trailing_singleton_ndims`, `logical_values`, `string_values`, `floating_tolerance`, `unary_signed_zero`, `scalar_left_divide`, `scalar_power_precedence`, `logical_short_circuit`, `conjugate_transpose`, `operator_implicit_expansion` |
| Arrays, shape, and indexing | 10 | `linear_indexing`, `colon_range`, `colon_matrix_indexing`, `empty_two_by_zero`, `empty_zero_by_three`, `size_numel_scalar_empty`, `size_dimension_queries`, `empty_concatenation_shape`, `array_copy_on_write`, `index_out_of_bounds_error` |
| Scripts, functions, and control flow | 16 | `if_elseif_else`, `if_array_condition`, `while_break_continue`, `for_range_control`, `for_matrix_columns`, `switch_selector_once`, `switch_cell_case_list`, `script_workspace_order`, `clear_workspace_and_locals`, `function_multiple_outputs`, `function_parameter_scope`, `function_early_return_outputs`, `function_nargin_nargout`, `function_variadic_arguments`, `function_nested_closure`, `syntax_command_form_common` |
| Command and session semantics | 4 | `command_semantics_bare_zero_arg`, `command_semantics_expression_arguments`, `command_semantics_workspace_commands`, `session_tic_toc` |
| Global and persistent storage | 7 | `global_missing_empty`, `global_preserves_existing`, `global_script_function_share`, `persistent_counter_initialization`, `persistent_function_isolation`, `persistent_conditional_declaration`, `persistent_array_repeated_update` |
| Function handles | 12 | `function_handle_named_file`, `function_handle_named_local`, `function_handle_anonymous_multi_input`, `function_handle_anonymous_capture`, `function_handle_anonymous_object_capture`, `function_handle_anonymous_multi_output`, `function_handle_anonymous_varargin_basic`, `function_handle_anonymous_varargin_nested_capture`, `function_handle_anonymous_varargin_positions`, `function_handle_anonymous_varargin_requested_outputs`, `function_handle_cell_call`, `function_dynamic_invocation` |
| Dynamic metaprogramming | 5 | `eval_current_workspace`, `scope_metaprogramming`, `eval_catch_forms`, `evalc_capture`, `last_error_state` |
| Core `classdef` | 37 | `value_class_copy`, `handle_class_alias`, `class_constructor`, `class_public_access`, `class_private_access_error`, `class_argument_semantics`, `class_inheritance_override`, `class_static_constant`, `class_dependent_property`, `class_access_controls`, `class_homogeneous_array`, `class_plus_overload`, `class_binary_operator_overloads`, `class_property_defaults`, `class_protected_access_error`, `class_reflection_predicates`, `class_abstract_dispatch`, `class_abstract_instantiation_error`, `class_abstract_missing_implementation_error`, `class_abstract_reflection`, `class_abstract_static_override`, `class_abstract_unimplemented_call_error`, `class_sealed_inheritance_error`, `class_sealed_metadata_dispatch`, `gc_lifecycle_acyclic_overwrite_clear`, `gc_lifecycle_constructor_failure`, `gc_lifecycle_cycle_root_survives`, `gc_lifecycle_cycle_unrooted`, `gc_lifecycle_destructor_error_automatic`, `gc_lifecycle_destructor_error_explicit`, `gc_lifecycle_error_unwind`, `gc_lifecycle_explicit_delete_alias`, `gc_lifecycle_function_return`, `gc_lifecycle_handle_array`, `gc_lifecycle_no_resurrection`, `gc_lifecycle_script_scope`, `gc_lifecycle_value_delete_method` |
| Exact character, string, and integer values | 14 | `char_integer_value_model_char`, `char_integer_value_model_string`, `char_integer_value_model_signed_bounds`, `char_integer_value_model_unsigned_bounds`, `char_integer_value_model_complex_integer`, `builtin_text_formatting`, `builtin_str2double`, `builtin_str2double_special`, `builtin_regexp`, `builtin_regexprep`, `string_cellstr`, `string_compare_predicates`, `string_find_replace`, `string_transform_utf16` |
| Cell arrays and structures | 10 | `aggregate_c1_empty_cells`, `aggregate_c1_empty_structs`, `aggregate_c1_nested_structs`, `aggregate_c1_scalar_access_write`, `aggregate_c2_colon_end`, `aggregate_c3_comma_assign`, `aggregate_c3_delete_growth`, `aggregate_c3_shape_error`, `aggregate_c3_transaction_rollback`, `aggregate_c3_writeback_cow` |
| Exact `single` values and operations | 6 | `single_conversion_rounding`, `single_complex_components`, `single_indexing_column_major`, `single_indexed_assignment_complex`, `single_transpose_conjugate`, `single_invalid_index_error` |
| Exception control flow | 12 | `try_catch_skip_handler`, `try_catch_plain_index_error`, `try_catch_binding_persists`, `try_catch_without_handler`, `try_catch_nested_routing`, `try_catch_callee_error`, `try_catch_write_retention`, `try_catch_binding_overwrite`, `builtin_error_assert`, `exception_rethrow`, `mexception_complete`, `builtin_warning_lastwarn` |
| Matrix division, power, and linear solve | 13 | `matrix_left_square_real_multi_rhs`, `matrix_left_overdetermined_real_multi_rhs`, `matrix_left_underdetermined_basic`, `matrix_left_complex_multi_rhs`, `matrix_scalar_and_one_by_one`, `matrix_empty_shapes`, `matrix_right_complex_relation`, `matrix_left_single_complex`, `matrix_left_shape_mismatch`, `matrix_right_shape_mismatch`, `matrix_integer_power`, `matrix_fractional_power_nonfinite`, `matrix_fractional_power_principal` |
| Core numeric built-ins | 74 | `builtin_elementary_arrays`, `builtin_elementary_complex`, `builtin_elementary_single`, `builtin_exp_log_angle_extended`, `builtin_math_constants_extended`, `builtin_math_binary_extended`, `builtin_math_unary_extended`, `builtin_math_extended_degrees`, `builtin_math_extended_reciprocal`, `builtin_math_extended_base2`, `builtin_bitwise_assumed_types`, `builtin_bitwise_binary_classes`, `builtin_bitwise_expansion_empty`, `builtin_bitwise_invalid_domain_error`, `builtin_bitwise_invalid_type_error`, `builtin_bitwise_shape_error`, `builtin_bitwise_shift_positions`, `builtin_predicates_extended`, `builtin_constructors_predicates_extended`, `builtin_array_accumulate_extended`, `builtin_array_transform_extended`, `builtin_array_set_extended`, `builtin_logical_find`, `builtin_matrix_rearrange`, `builtin_matrix_functions_extended`, `builtin_matrix_functions_classes`, `builtin_matrix_functions_empty`, `builtin_norm_matrix_orders`, `builtin_norm_single_complex`, `builtin_arithmetic_reductions_extended`, `builtin_arithmetic_reductions_classes`, `builtin_extrema_reductions_extended`, `builtin_extrema_elementwise_extended`, `builtin_extrema_complex_comparison`, `builtin_extrema_classes`, `builtin_fft_family_values`, `builtin_fft_family_shapes`, `builtin_fft_family_shift_empty`, `builtin_concat_classes`, `builtin_concat_column_major`, `builtin_sequences_round`, `builtin_sequences_round_single`, `builtin_sort_classes`, `builtin_sort_order`, `builtin_statistics_classes`, `builtin_statistics_double`, `builtin_statistics_mode_extended`, `builtin_statistics_range_bounds_extended`, `builtin_statistics_cov_extended`, `builtin_statistics_corrcoef_extended`, `builtin_chol_status`, `builtin_factorization_single_complex`, `builtin_lu_det`, `builtin_qr_factorization`, `builtin_sqrtm_outputs`, `builtin_sqrtm_principal`, `builtin_eig_single_left`, `builtin_eig_spectral`, `builtin_rank_cond`, `builtin_rank_cond_single`, `builtin_svd_single_complex`, `builtin_svd_spectral`, `core_pi_resolution`, `builtin_coordinate_transforms`, `builtin_unwrap`, `builtin_interp1_linear_nearest`, `builtin_interp1_shapes_classes`, `builtin_polynomial_calculus`, `builtin_polynomial_classes_empty`, `builtin_polynomial_convolution_values`, `builtin_polynomial_roots_provider`, `builtin_array_windows_cumulative`, `builtin_array_windows_moving`, `builtin_array_windows_rearrange` |
| Sparse matrices | 1 | `builtin_sparse_core_semantics` |
| Set, ordering, and numeric calculus | 3 | `builtin_set_calculus_sets`, `builtin_set_calculus_ordering`, `builtin_set_calculus_numeric` |
| Type and container conversions | 10 | `builtin_conversion_extended_cast_like_complex`, `builtin_conversion_extended_typecast_native_bytes`, `builtin_conversion_extended_typecast_empty_shape`, `builtin_conversion_extended_num2cell_order`, `builtin_conversion_extended_mat2cell_roundtrip`, `builtin_conversion_extended_cell2mat_mixed_error`, `builtin_conversion_extended_radix_encode`, `builtin_conversion_extended_radix_decode`, `builtin_conversion_extended_dec2bin_signed`, `builtin_conversion_extended_bin2dec_cellstr` |
| File I/O | 2 | `builtin_file_io_text_matrix`, `builtin_filesystem_paths` |
| Tables | 3 | `table_construct_index`, `table_cow_schema_mutation`, `table_readtable_text_inference` |
| Graphics | 44 | `graphics_plot_matrix_handle_vector`, `graphics_plot_empty_handle_vector`, `graphics_hold_state_transitions`, `graphics_hold_preserves_children`, `graphics_cla_preserves_axes_hold`, `graphics_clf_resets_current_hold`, `graphics_close_new_current_unheld`, `graphics_command_figure_close_all`, `graphics_2d_get_ambiguous_property`, `graphics_2d_hold_color_order_style`, `graphics_2d_legend_grid`, `graphics_2d_limits_auto_reacts`, `graphics_2d_limits_manual`, `graphics_2d_plot_linespec_namevalue`, `graphics_2d_plot_linespec_properties`, `graphics_2d_replace_invalidates_lines`, `graphics_2d_scatter_scalar_properties`, `graphics_2d_set_get_property_names`, `graphics_2d_set_invalid_linestyle`, `graphics_2d_set_invalid_linewidth`, `graphics_2d_text_objects`, `graphics_2d_axes_style_box`, `graphics_2d_axes_ticks_modes`, `graphics_2d_interpreter_legend_order`, `graphics_2d_marker_indices_semantics`, `graphics_2d_high_frequency_chart_classes`, `graphics_2d_contour_image_semantics`, `graphics_2d_polar_semantics`, `graphics_multi_axes_layout`, `graphics_3d_surf_defaults`, `graphics_3d_view_zlim_label`, `graphics_3d_mesh_defaults`, `graphics_3d_plot3_defaults`, `graphics_3d_scatter3_defaults`, `graphics_3d_scatter3_scalar_cdata`, `graphics_3d_axes_aspect_camera`, `graphics_colormap_parula_r2022b`, `graphics_colormap_catalog_r2022b`, `graphics_colorbar_ticks_r2022b`, `graphics_patch_xy_defaults`, `graphics_patch_faces_interp`, `graphics_patch_derived_functions`, `graphics_property_figure_posthoc`, `graphics_property_chart_posthoc` |

These references establish MATLAB oracle behavior only. A passing oracle run
does not claim that OpenMat can execute or pass any of the cases; that requires
an OpenMat differential runner to produce and compare its own observations.

Every case is state-independent and must not rely on execution order. Storage
cases use unique global and local-function names. The OpenMat runner starts a
fresh process per case; the MATLAB oracle uses a fresh case function workspace
inside its selected batch, so unique names also prevent process-global or
function-persistent state from coupling cases. The current schemas observe only
`openmat_result` or an error category; they do not contain warning fields or an
independent hidden-storage snapshot.

## OpenMat runner

Run the OpenMat-owned engine against every manifest from the repository root:

```powershell
pwsh -NoProfile -File tools/openmat-conformance/Invoke-OpenMatConformance.ps1
```

The runner builds `openmat-cli` unless `-OpenMatPath` names an existing binary.
It invokes `openmat-cli conformance <manifest.json>` in a fresh process for each
case, writes one observation in the manifest-selected schema version per case plus
`run-summary.json`, and compares each observation with both the manifest
expectation and the checked-in R2022b reference. The default result directory
is a unique directory under the operating-system temporary directory; use
`-ResultDirectory` to preserve it elsewhere. The runner refuses to write into
the case tree or checked-in reference directory.

Use `-Case scalar_shape` for one case, comma-separated names or wildcards for a
selection, `-List` to inspect that selection, and `-JsonSummary` for a compact
machine-readable summary on stdout. Comparator fixtures run independently:

```powershell
pwsh -NoProfile -File tools/openmat-conformance/tests/Test-Comparator.ps1
```

Exit code `0` means all selected cases passed, `2` means at least one semantic
comparison failed, `3` means there were unsupported cases but no failures or
internal errors, and `4` means a per-case runner/internal failure occurred.
Setup, selection, or path errors return `64`; internal status takes precedence,
then failure, then unsupported.

The CLI canonicalizes each entry program, configures the RuntimeEngine with the
entry directory followed by `cases/support`, and executes it through a File-mode
Kernel request. The RuntimeEngine parses and compiles each resolved function,
script, or class as its own source unit; it does not concatenate support source
text. Entry-directory definitions take precedence over support definitions.
Payloads larger than the kernel's bounded full-preview limit, or value classes
that the kernel cannot transfer losslessly, use `unsupported-payload`, which
counts as `unsupported` in the summary.

## Recorded R2022b run

The checked-in reference observations are produced from the repository root
with PowerShell 7 and the local licensed MATLAB R2022b installation:

```powershell
pwsh -NoProfile -File tools/matlab-oracle/Invoke-MatlabOracle.ps1 `
    -MatlabPath $env:MATLAB_EXE `
    -ResultDirectory tests/conformance/reference/matlab-r2022b
```

The focused command used for the seven global/persistent observations is:

```powershell
pwsh -NoProfile -File tools/matlab-oracle/Invoke-MatlabOracle.ps1 `
    -MatlabPath $env:MATLAB_EXE `
    -Tag global-persistent `
    -ResultDirectory tests/conformance/reference/matlab-r2022b
```

Machine paths and raw MATLAB output are not written into observations or
expectations. `run-summary.json` is intentionally ignored because timings vary.
