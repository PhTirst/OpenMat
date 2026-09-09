#![doc = "The source-callable core built-in function set for release one."]
#![forbid(unsafe_code)]

mod argument_validation;
mod core_accumulate;
mod core_array_transform;
mod core_array_windows;
mod core_bitwise;
mod core_concat;
mod core_constants;
mod core_conversion_extended;
mod core_coordinates;
mod core_elementary;
mod core_fft;
mod core_find;
mod core_isosurface;
mod core_linalg;
mod core_logical_reduction;
mod core_math;
mod core_math_extended;
mod core_matrix;
mod core_numeric;
mod core_polynomial_extended;
mod core_predicates;
mod core_programming;
mod core_random;
mod core_rearrange;
mod core_reductions;
mod core_regex;
mod core_round;
mod core_sequences;
mod core_set;
mod core_set_calculus;
mod core_shape;
mod core_sort;
mod core_sparse;
mod core_spectral;
mod core_statistics;
mod core_string;
mod core_text_conversion;
mod file_io;
mod filesystem_extended;
mod graphics;
pub mod quadrature;
mod session;
mod stream_io;
mod table;
mod textscan;

use openmat_array::{
    ArrayData, CharCodeUnit, ComplexInteger, DenseArray, IntegerArrayData, IntegerComponent,
    IntegerElement, Shape,
};
use openmat_runtime::{
    BuiltinContext, BuiltinError, BuiltinErrorCategory, BuiltinRegistrationError, BuiltinResult,
    OutputEvent,
};
use openmat_value::{
    CellArray, FieldName, StringArray, StringElement, StringValue, StructArray, StructSchema, Value,
};

pub use openmat_runtime::BuiltinRegistry;

const U64_EXCLUSIVE_UPPER_BOUND: f64 = 18_446_744_073_709_551_616.0;

/// Creates a registry containing the currently implemented release-one core.
///
/// It is not presented as a complete MATLAB standard library. In particular,
/// unsupported overloads are rejected by the individual built-ins instead of
/// being silently approximated.
///
/// # Errors
///
/// Returns a registration error if the registry's handle space is exhausted.
pub fn minimal_registry() -> Result<BuiltinRegistry, BuiltinRegistrationError> {
    let mut registry = BuiltinRegistry::new();
    register_minimal(&mut registry)?;
    Ok(registry)
}

/// Adds the minimal built-in set to an existing registry.
///
/// # Errors
///
/// Returns an error for a duplicate name or exhausted registry handle space.
/// Registrations completed before an error remain installed.
#[allow(clippy::too_many_lines)]
pub fn register_minimal(registry: &mut BuiltinRegistry) -> Result<(), BuiltinRegistrationError> {
    argument_validation::register(registry)?;
    core_sparse::register_sparse_builtins(registry)?;
    registry.register_bare_callable("pi", core_constants::pi_builtin)?;
    register_extended_constants(registry)?;
    registry.register("class", class_builtin)?;
    registry.register("isa", isa_builtin)?;
    registry.register("size", size_builtin)?;
    registry.register("numel", numel_builtin)?;
    registry.register("ndims", core_shape::ndims_builtin)?;
    registry.register("length", core_shape::length_builtin)?;
    registry.register("isempty", core_shape::isempty_builtin)?;
    register_predicates(registry)?;
    registry.register("zeros", core_shape::zeros_builtin)?;
    registry.register("ones", core_shape::ones_builtin)?;
    registry.register("eye", core_shape::eye_builtin)?;
    registry.register("rand", core_random::rand_builtin)?;
    registry.register("randn", core_random::randn_builtin)?;
    registry.register("randi", core_random::randi_builtin)?;
    registry.register("randperm", core_random::randperm_builtin)?;
    registry.register("rng", core_random::rng_builtin)?;
    registry.register("reshape", core_shape::reshape_builtin)?;
    registry.register("squeeze", core_rearrange::squeeze_builtin)?;
    registry.register("permute", core_rearrange::permute_builtin)?;
    registry.register("repmat", core_rearrange::repmat_builtin)?;
    registry.register("logical", core_numeric::logical_builtin)?;
    registry.register("double", core_numeric::double_builtin)?;
    registry.register("single", core_numeric::single_builtin)?;
    registry.register("char", core_numeric::char_builtin)?;
    registry.register("int8", core_numeric::int8_builtin)?;
    registry.register("uint8", core_numeric::uint8_builtin)?;
    registry.register("int16", core_numeric::int16_builtin)?;
    registry.register("uint16", core_numeric::uint16_builtin)?;
    registry.register("int32", core_numeric::int32_builtin)?;
    registry.register("uint32", core_numeric::uint32_builtin)?;
    registry.register("int64", core_numeric::int64_builtin)?;
    registry.register("uint64", core_numeric::uint64_builtin)?;
    registry.register("intmin", intmin_builtin)?;
    registry.register("intmax", intmax_builtin)?;
    registry.register("real", core_numeric::real_builtin)?;
    registry.register("imag", core_numeric::imag_builtin)?;
    registry.register("conj", core_numeric::conj_builtin)?;
    registry.register("complex", complex_builtin)?;
    register_extended_conversions(registry)?;
    registry.register("string", string_builtin)?;
    registry.register("strings", strings_builtin)?;
    registry.register("cell", cell_builtin)?;
    registry.register("struct", struct_builtin)?;
    register_table_functions(registry)?;
    registry.register("deal", deal_builtin)?;
    registry.register_bare_callable("missing", missing_builtin)?;
    registry.register("sum", core_reductions::sum_builtin)?;
    registry.register("prod", core_reductions::prod_builtin)?;
    registry.register("min", core_reductions::min_builtin)?;
    registry.register("max", core_reductions::max_builtin)?;
    registry.register("dot", core_numeric::dot_builtin)?;
    registry.register("norm", core_numeric::norm_builtin)?;
    registry.register("any", core_logical_reduction::any_builtin)?;
    registry.register("all", core_logical_reduction::all_builtin)?;
    registry.register("find", core_find::find_builtin)?;
    registry.register("diag", core_matrix::diag_builtin)?;
    registry.register("triu", core_matrix::triu_builtin)?;
    registry.register("tril", core_matrix::tril_builtin)?;
    registry.register("trace", core_matrix::trace_builtin)?;
    registry.register("cross", core_matrix::cross_builtin)?;
    registry.register("kron", core_matrix::kron_builtin)?;
    registry.register("meshgrid", core_matrix::meshgrid_builtin)?;
    registry.register("isosurface", core_isosurface::isosurface_builtin)?;
    register_common_array_functions(registry)?;
    register_array_window_functions(registry)?;
    registry.register("fix", core_elementary::fix_builtin)?;
    registry.register("floor", core_elementary::floor_builtin)?;
    registry.register("ceil", core_elementary::ceil_builtin)?;
    registry.register("exp", core_elementary::exp_builtin)?;
    registry.register("log", core_elementary::log_builtin)?;
    registry.register("log10", core_elementary::log10_builtin)?;
    registry.register("sin", core_elementary::sin_builtin)?;
    registry.register("cos", core_elementary::cos_builtin)?;
    registry.register("tan", core_elementary::tan_builtin)?;
    register_extended_math(registry)?;
    registry.register("round", core_round::round_builtin)?;
    registry.register("linspace", core_sequences::linspace_builtin)?;
    registry.register("logspace", core_sequences::logspace_builtin)?;
    registry.register("quadgk", quadrature::quadgk_builtin)?;
    register_statistics(registry)?;
    registry.register("sort", core_sort::sort_builtin)?;
    registry.register("cat", core_concat::cat_builtin)?;
    registry.register("horzcat", core_concat::horzcat_builtin)?;
    registry.register("vertcat", core_concat::vertcat_builtin)?;
    register_linear_algebra_functions(registry)?;
    core_polynomial_extended::register_polynomial_extended(registry)?;
    register_fourier_functions(registry)?;
    register_coordinate_functions(registry)?;
    register_text_functions(registry)?;
    register_programming_functions(registry)?;
    registry.register("abs", core_elementary::abs_builtin)?;
    registry.register("sqrt", core_elementary::sqrt_builtin)?;
    register_predefined_colormaps(registry)?;
    registry.register("disp", disp_builtin)?;
    register_session_commands(registry)?;
    register_file_io(registry)?;
    filesystem_extended::register_filesystem_extended(registry)?;
    register_graphics(registry)?;
    Ok(())
}

fn register_statistics(registry: &mut BuiltinRegistry) -> Result<(), BuiltinRegistrationError> {
    registry.register("mean", core_statistics::mean_builtin)?;
    registry.register("median", core_statistics::median_builtin)?;
    registry.register("mode", core_statistics::mode_builtin)?;
    registry.register("range", core_statistics::range_builtin)?;
    registry.register("bounds", core_statistics::bounds_builtin)?;
    registry.register("std", core_statistics::std_builtin)?;
    registry.register("var", core_statistics::var_builtin)?;
    registry.register("cov", core_statistics::cov_builtin)?;
    registry.register("corrcoef", core_statistics::corrcoef_builtin)?;
    Ok(())
}

fn register_linear_algebra_functions(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register("lu", core_linalg::lu_builtin)?;
    registry.register("qr", core_linalg::qr_builtin)?;
    registry.register("chol", core_linalg::chol_builtin)?;
    registry.register("det", core_linalg::det_builtin)?;
    registry.register("inv", core_linalg::inv_builtin)?;
    registry.register("sqrtm", core_linalg::sqrtm_builtin)?;
    registry.register("svd", core_spectral::svd_builtin)?;
    registry.register("pinv", core_spectral::pinv_builtin)?;
    registry.register("eig", core_spectral::eig_builtin)?;
    registry.register("rank", core_spectral::rank_builtin)?;
    registry.register("cond", core_spectral::cond_builtin)?;
    Ok(())
}

fn register_fourier_functions(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register("fft", core_fft::fft_builtin)?;
    registry.register("ifft", core_fft::ifft_builtin)?;
    registry.register("fft2", core_fft::fft2_builtin)?;
    registry.register("ifft2", core_fft::ifft2_builtin)?;
    registry.register("fftn", core_fft::fftn_builtin)?;
    registry.register("ifftn", core_fft::ifftn_builtin)?;
    registry.register("fftshift", core_array_transform::fftshift_builtin)?;
    registry.register("ifftshift", core_array_transform::ifftshift_builtin)?;
    Ok(())
}

fn register_coordinate_functions(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register("cart2pol", core_coordinates::cart2pol_builtin)?;
    registry.register("pol2cart", core_coordinates::pol2cart_builtin)?;
    registry.register("cart2sph", core_coordinates::cart2sph_builtin)?;
    registry.register("sph2cart", core_coordinates::sph2cart_builtin)?;
    registry.register("unwrap", core_coordinates::unwrap_builtin)?;
    Ok(())
}

fn register_text_functions(registry: &mut BuiltinRegistry) -> Result<(), BuiltinRegistrationError> {
    registry.register("strcmp", core_string::strcmp_builtin)?;
    registry.register("strcmpi", core_string::strcmpi_builtin)?;
    registry.register("strncmp", core_string::strncmp_builtin)?;
    registry.register("strncmpi", core_string::strncmpi_builtin)?;
    registry.register("strlength", core_string::strlength_builtin)?;
    registry.register("upper", core_string::upper_builtin)?;
    registry.register("lower", core_string::lower_builtin)?;
    registry.register("reverse", core_string::reverse_builtin)?;
    registry.register("strip", core_string::strip_builtin)?;
    registry.register("strtrim", core_string::strtrim_builtin)?;
    registry.register("deblank", core_string::deblank_builtin)?;
    registry.register("contains", core_string::contains_builtin)?;
    registry.register("startsWith", core_string::starts_with_builtin)?;
    registry.register("endsWith", core_string::ends_with_builtin)?;
    registry.register("strfind", core_string::strfind_builtin)?;
    registry.register("strrep", core_string::strrep_builtin)?;
    registry.register("erase", core_string::erase_builtin)?;
    registry.register("replace", core_string::replace_builtin)?;
    registry.register("sprintf", core_text_conversion::sprintf_builtin)?;
    registry.register("num2str", core_text_conversion::num2str_builtin)?;
    registry.register("str2double", core_text_conversion::str2double_builtin)?;
    registry.register("regexp", core_regex::regexp_builtin)?;
    registry.register("regexpi", core_regex::regexpi_builtin)?;
    registry.register("regexprep", core_regex::regexprep_builtin)?;
    Ok(())
}

fn register_programming_functions(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register("error", core_programming::error_builtin)?;
    registry.register("assert", core_programming::assert_builtin)?;
    registry.register("warning", core_programming::warning_builtin)?;
    registry.register("lastwarn", core_programming::lastwarn_builtin)?;
    Ok(())
}

fn register_file_io(registry: &mut BuiltinRegistry) -> Result<(), BuiltinRegistrationError> {
    registry.register("fopen", stream_io::fopen_builtin)?;
    registry.register("fclose", stream_io::fclose_builtin)?;
    registry.register("fread", stream_io::fread_builtin)?;
    registry.register("fwrite", stream_io::fwrite_builtin)?;
    registry.register("fseek", stream_io::fseek_builtin)?;
    registry.register("ftell", stream_io::ftell_builtin)?;
    registry.register("textscan", textscan::textscan_builtin)?;
    registry.register("load", file_io::load_builtin)?;
    registry.register("save", file_io::save_builtin)?;
    registry.register("fileread", file_io::fileread_builtin)?;
    registry.register("readmatrix", file_io::readmatrix_builtin)?;
    registry.register("writematrix", file_io::writematrix_builtin)?;
    registry.register("readtable", file_io::readtable_builtin)?;
    registry.register("writetable", file_io::writetable_builtin)?;
    Ok(())
}

fn register_table_functions(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register("table", table::table_builtin)?;
    registry.register("array2table", table::array2table_builtin)?;
    registry.register("cell2table", table::cell2table_builtin)?;
    registry.register("struct2table", table::struct2table_builtin)?;
    registry.register("table2array", table::table2array_builtin)?;
    registry.register("table2cell", table::table2cell_builtin)?;
    registry.register("table2struct", table::table2struct_builtin)?;
    registry.register("height", table::height_builtin)?;
    registry.register("width", table::width_builtin)?;
    registry.register("istable", table::istable_builtin)?;
    registry.register("head", table::head_builtin)?;
    registry.register("tail", table::tail_builtin)?;
    registry.register("sortrows", table::sortrows_builtin)?;
    registry.register("ismissing", table::ismissing_builtin)?;
    registry.register("rmmissing", table::rmmissing_builtin)?;
    registry.register("fillmissing", table::fillmissing_builtin)?;
    registry.register("innerjoin", table::innerjoin_builtin)?;
    registry.register("outerjoin", table::outerjoin_builtin)?;
    registry.register("groupcounts", table::groupcounts_builtin)?;
    registry.register("groupsummary", table::groupsummary_builtin)?;
    Ok(())
}

fn register_predefined_colormaps(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register("parula", graphics::parula_builtin)?;
    registry.register("turbo", graphics::turbo_builtin)?;
    registry.register("hsv", graphics::hsv_builtin)?;
    registry.register("hot", graphics::hot_builtin)?;
    registry.register("cool", graphics::cool_builtin)?;
    registry.register("spring", graphics::spring_builtin)?;
    registry.register("summer", graphics::summer_builtin)?;
    registry.register("autumn", graphics::autumn_builtin)?;
    registry.register("winter", graphics::winter_builtin)?;
    registry.register("gray", graphics::gray_builtin)?;
    registry.register("bone", graphics::bone_builtin)?;
    registry.register("copper", graphics::copper_builtin)?;
    registry.register("pink", graphics::pink_builtin)?;
    registry.register("jet", graphics::jet_builtin)?;
    registry.register("lines", graphics::lines_builtin)?;
    registry.register("colorcube", graphics::colorcube_builtin)?;
    registry.register("prism", graphics::prism_builtin)?;
    registry.register("flag", graphics::flag_builtin)?;
    registry.register("white", graphics::white_builtin)?;
    registry.register("vga", graphics::vga_builtin)?;
    Ok(())
}

fn register_session_commands(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register("clc", session::clc_builtin)?;
    registry.register_bare_callable("cd", session::cd_builtin)?;
    registry.register_bare_callable("pwd", session::pwd_builtin)?;
    registry.register("format", session::format_builtin)?;
    registry.register("who", session::who_builtin)?;
    registry.register("whos", session::whos_builtin)?;
    registry.register("__openmat_exist_var", session::exist_var_builtin)?;
    Ok(())
}

fn register_extended_constants(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register_bare_callable("i", core_constants::i_builtin)?;
    registry.register_bare_callable("j", core_constants::j_builtin)?;
    registry.register_bare_callable("true", core_shape::true_builtin)?;
    registry.register_bare_callable("false", core_shape::false_builtin)?;
    registry.register_bare_callable("eps", core_constants::eps_builtin)?;
    registry.register_bare_callable("Inf", core_constants::inf_builtin)?;
    registry.register_bare_callable("inf", core_constants::inf_builtin)?;
    registry.register_bare_callable("NaN", core_constants::nan_builtin)?;
    registry.register_bare_callable("nan", core_constants::nan_builtin)?;
    registry.register_bare_callable("realmin", core_constants::realmin_builtin)?;
    registry.register_bare_callable("realmax", core_constants::realmax_builtin)?;
    registry.register_bare_callable("flintmax", core_constants::flintmax_builtin)?;
    Ok(())
}

fn register_extended_math(registry: &mut BuiltinRegistry) -> Result<(), BuiltinRegistrationError> {
    registry.register("expm1", core_elementary::expm1_builtin)?;
    registry.register("log1p", core_elementary::log1p_builtin)?;
    registry.register("sign", core_math::sign_builtin)?;
    registry.register("angle", core_math::angle_builtin)?;
    registry.register("mod", core_math::mod_builtin)?;
    registry.register("rem", core_math::rem_builtin)?;
    registry.register("hypot", core_math::hypot_builtin)?;
    registry.register("asin", core_math::asin_builtin)?;
    registry.register("acos", core_math::acos_builtin)?;
    registry.register("atan", core_math::atan_builtin)?;
    registry.register("atan2", core_math::atan2_builtin)?;
    registry.register("sinh", core_math::sinh_builtin)?;
    registry.register("cosh", core_math::cosh_builtin)?;
    registry.register("tanh", core_math::tanh_builtin)?;
    registry.register("asinh", core_math::asinh_builtin)?;
    registry.register("acosh", core_math::acosh_builtin)?;
    registry.register("atanh", core_math::atanh_builtin)?;
    registry.register("deg2rad", core_math::deg2rad_builtin)?;
    registry.register("rad2deg", core_math::rad2deg_builtin)?;
    registry.register("sind", core_math_extended::sind_builtin)?;
    registry.register("cosd", core_math_extended::cosd_builtin)?;
    registry.register("tand", core_math_extended::tand_builtin)?;
    registry.register("asind", core_math_extended::asind_builtin)?;
    registry.register("acosd", core_math_extended::acosd_builtin)?;
    registry.register("atand", core_math_extended::atand_builtin)?;
    registry.register("atan2d", core_math_extended::atan2d_builtin)?;
    registry.register("sec", core_math_extended::sec_builtin)?;
    registry.register("csc", core_math_extended::csc_builtin)?;
    registry.register("cot", core_math_extended::cot_builtin)?;
    registry.register("sech", core_math_extended::sech_builtin)?;
    registry.register("csch", core_math_extended::csch_builtin)?;
    registry.register("coth", core_math_extended::coth_builtin)?;
    registry.register("asec", core_math_extended::asec_builtin)?;
    registry.register("acsc", core_math_extended::acsc_builtin)?;
    registry.register("acot", core_math_extended::acot_builtin)?;
    registry.register("asech", core_math_extended::asech_builtin)?;
    registry.register("acsch", core_math_extended::acsch_builtin)?;
    registry.register("acoth", core_math_extended::acoth_builtin)?;
    registry.register("log2", core_math_extended::log2_builtin)?;
    registry.register("pow2", core_math_extended::pow2_builtin)?;
    registry.register("nextpow2", core_math_extended::nextpow2_builtin)?;
    core_bitwise::register_bitwise(registry)?;
    Ok(())
}

fn register_extended_conversions(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register("cast", core_conversion_extended::cast_builtin)?;
    registry.register("typecast", core_conversion_extended::typecast_builtin)?;
    registry.register("num2cell", core_conversion_extended::num2cell_builtin)?;
    registry.register("cell2mat", core_conversion_extended::cell2mat_builtin)?;
    registry.register("mat2cell", core_conversion_extended::mat2cell_builtin)?;
    registry.register("dec2hex", core_conversion_extended::dec2hex_builtin)?;
    registry.register("hex2dec", core_conversion_extended::hex2dec_builtin)?;
    registry.register("dec2bin", core_conversion_extended::dec2bin_builtin)?;
    registry.register("bin2dec", core_conversion_extended::bin2dec_builtin)?;
    Ok(())
}

fn register_predicates(registry: &mut BuiltinRegistry) -> Result<(), BuiltinRegistrationError> {
    registry.register("isnan", core_predicates::isnan_builtin)?;
    registry.register("isinf", core_predicates::isinf_builtin)?;
    registry.register("isfinite", core_predicates::isfinite_builtin)?;
    registry.register("isreal", core_predicates::isreal_builtin)?;
    registry.register("isnumeric", core_predicates::isnumeric_builtin)?;
    registry.register("isfloat", core_predicates::isfloat_builtin)?;
    registry.register("isinteger", core_predicates::isinteger_builtin)?;
    registry.register("islogical", core_predicates::islogical_builtin)?;
    registry.register("ischar", core_predicates::ischar_builtin)?;
    registry.register("isstring", core_predicates::isstring_builtin)?;
    registry.register("iscell", core_predicates::iscell_builtin)?;
    registry.register("isstruct", core_predicates::isstruct_builtin)?;
    registry.register("isobject", core_predicates::isobject_builtin)?;
    registry.register("isscalar", core_predicates::isscalar_builtin)?;
    registry.register("isvector", core_predicates::isvector_builtin)?;
    registry.register("ismatrix", core_predicates::ismatrix_builtin)?;
    registry.register("isrow", core_predicates::isrow_builtin)?;
    registry.register("iscolumn", core_predicates::iscolumn_builtin)?;
    registry.register("isequal", core_predicates::isequal_builtin)?;
    registry.register("isequaln", core_predicates::isequaln_builtin)?;
    Ok(())
}

fn register_common_array_functions(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register("diff", core_accumulate::diff_builtin)?;
    registry.register("cumsum", core_accumulate::cumsum_builtin)?;
    registry.register("cumprod", core_accumulate::cumprod_builtin)?;
    registry.register("ndgrid", core_array_transform::ndgrid_builtin)?;
    registry.register("flip", core_array_transform::flip_builtin)?;
    registry.register("fliplr", core_array_transform::fliplr_builtin)?;
    registry.register("flipud", core_array_transform::flipud_builtin)?;
    registry.register("circshift", core_array_transform::circshift_builtin)?;
    registry.register("unique", core_set::unique_builtin)?;
    registry.register("ismember", core_set::ismember_builtin)?;
    registry.register_bare_callable("intersect", core_set_calculus::intersect_builtin)?;
    registry.register_bare_callable("union", core_set_calculus::union_builtin)?;
    registry.register_bare_callable("setdiff", core_set_calculus::setdiff_builtin)?;
    registry.register_bare_callable("setxor", core_set_calculus::setxor_builtin)?;
    registry.register_bare_callable("issorted", core_set_calculus::issorted_builtin)?;
    registry.register_bare_callable("mink", core_set_calculus::mink_builtin)?;
    registry.register_bare_callable("maxk", core_set_calculus::maxk_builtin)?;
    registry.register_bare_callable("trapz", core_set_calculus::trapz_builtin)?;
    registry.register_bare_callable("cumtrapz", core_set_calculus::cumtrapz_builtin)?;
    registry.register_bare_callable("gradient", core_set_calculus::gradient_builtin)?;
    Ok(())
}

fn register_array_window_functions(
    registry: &mut BuiltinRegistry,
) -> Result<(), BuiltinRegistrationError> {
    registry.register("ipermute", core_array_windows::ipermute_builtin)?;
    registry.register("rot90", core_array_windows::rot90_builtin)?;
    registry.register("shiftdim", core_array_windows::shiftdim_builtin)?;
    registry.register("repelem", core_array_windows::repelem_builtin)?;
    registry.register("blkdiag", core_array_windows::blkdiag_builtin)?;
    registry.register("cummin", core_array_windows::cummin_builtin)?;
    registry.register("cummax", core_array_windows::cummax_builtin)?;
    registry.register("movsum", core_array_windows::movsum_builtin)?;
    registry.register("movmean", core_array_windows::movmean_builtin)?;
    registry.register("movmin", core_array_windows::movmin_builtin)?;
    registry.register("movmax", core_array_windows::movmax_builtin)?;
    Ok(())
}

fn register_graphics(registry: &mut BuiltinRegistry) -> Result<(), BuiltinRegistrationError> {
    registry.register_bare_callable("figure", graphics::figure_builtin)?;
    registry.register("axes", graphics::axes_builtin)?;
    registry.register("polaraxes", graphics::polaraxes_builtin)?;
    registry.register("subplot", graphics::subplot_builtin)?;
    registry.register("tiledlayout", graphics::tiledlayout_builtin)?;
    registry.register("nexttile", graphics::nexttile_builtin)?;
    registry.register_bare_callable("gcf", graphics::gcf_builtin)?;
    registry.register_bare_callable("gca", graphics::gca_builtin)?;
    registry.register_bare_callable("groot", graphics::groot_builtin)?;
    registry.register("plot", graphics::plot_builtin)?;
    registry.register("polarplot", graphics::polarplot_builtin)?;
    registry.register("stairs", graphics::stairs_builtin)?;
    registry.register("stem", graphics::stem_builtin)?;
    registry.register("errorbar", graphics::errorbar_builtin)?;
    registry.register("area", graphics::area_builtin)?;
    registry.register("bar", graphics::bar_builtin)?;
    registry.register("histogram", graphics::histogram_builtin)?;
    registry.register("contour", graphics::contour_builtin)?;
    registry.register("contourf", graphics::contourf_builtin)?;
    registry.register("image", graphics::image_builtin)?;
    registry.register("imagesc", graphics::imagesc_builtin)?;
    registry.register("semilogx", graphics::semilogx_builtin)?;
    registry.register("semilogy", graphics::semilogy_builtin)?;
    registry.register("loglog", graphics::loglog_builtin)?;
    registry.register("plot3", graphics::plot3_builtin)?;
    registry.register("scatter", graphics::scatter_builtin)?;
    registry.register("scatter3", graphics::scatter3_builtin)?;
    registry.register("surf", graphics::surf_builtin)?;
    registry.register("mesh", graphics::mesh_builtin)?;
    registry.register("patch", graphics::patch_builtin)?;
    registry.register("fill", graphics::fill_builtin)?;
    registry.register("fill3", graphics::fill3_builtin)?;
    registry.register("trisurf", graphics::trisurf_builtin)?;
    registry.register("trimesh", graphics::trimesh_builtin)?;
    registry.register_bare_callable("shading", graphics::shading_builtin)?;
    registry.register_bare_callable("camlight", graphics::camlight_builtin)?;
    registry.register_bare_callable("lighting", graphics::lighting_builtin)?;
    registry.register("isonormals", graphics::isonormals_builtin)?;
    registry.register_bare_callable("colorbar", graphics::colorbar_builtin)?;
    registry.register_bare_callable("colormap", graphics::colormap_builtin)?;
    registry.register_bare_callable("view", graphics::view_builtin)?;
    registry.register_bare_callable("axis", graphics::axis_builtin)?;
    registry.register_bare_callable("daspect", graphics::daspect_builtin)?;
    registry.register_bare_callable("pbaspect", graphics::pbaspect_builtin)?;
    registry.register("hold", graphics::hold_builtin)?;
    registry.register_bare_callable("ishold", graphics::ishold_builtin)?;
    registry.register("isgraphics", graphics::isgraphics_builtin)?;
    registry.register("get", graphics::get_builtin)?;
    registry.register("set", graphics::set_builtin)?;
    registry.register("title", graphics::title_builtin)?;
    registry.register("xlabel", graphics::xlabel_builtin)?;
    registry.register("ylabel", graphics::ylabel_builtin)?;
    registry.register("zlabel", graphics::zlabel_builtin)?;
    registry.register("legend", graphics::legend_builtin)?;
    registry.register("xlim", graphics::xlim_builtin)?;
    registry.register("ylim", graphics::ylim_builtin)?;
    registry.register("zlim", graphics::zlim_builtin)?;
    registry.register("thetalim", graphics::thetalim_builtin)?;
    registry.register("rlim", graphics::rlim_builtin)?;
    registry.register_bare_callable("clim", graphics::clim_builtin)?;
    registry.register_bare_callable("caxis", graphics::caxis_builtin)?;
    registry.register_bare_callable("xticks", graphics::xticks_builtin)?;
    registry.register_bare_callable("xticklabels", graphics::xticklabels_builtin)?;
    registry.register_bare_callable("yticks", graphics::yticks_builtin)?;
    registry.register_bare_callable("yticklabels", graphics::yticklabels_builtin)?;
    registry.register_bare_callable("zticks", graphics::zticks_builtin)?;
    registry.register_bare_callable("zticklabels", graphics::zticklabels_builtin)?;
    registry.register_bare_callable("thetaticks", graphics::thetaticks_builtin)?;
    registry.register_bare_callable("rticks", graphics::rticks_builtin)?;
    registry.register_bare_callable("thetaticklabels", graphics::thetaticklabels_builtin)?;
    registry.register_bare_callable("rticklabels", graphics::rticklabels_builtin)?;
    registry.register("grid", graphics::grid_builtin)?;
    registry.register_bare_callable("box", graphics::box_builtin)?;
    registry.register("cla", graphics::cla_builtin)?;
    registry.register("clf", graphics::clf_builtin)?;
    registry.register("close", graphics::close_builtin)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntegerClass {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
}

fn intmin_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    integer_limit_builtin("intmin", arguments, context, false)
}

fn intmax_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    integer_limit_builtin("intmax", arguments, context, true)
}

fn integer_limit_builtin(
    name: &'static str,
    arguments: &[Value],
    context: &mut BuiltinContext<'_>,
    maximum: bool,
) -> BuiltinResult {
    expect_argument_count(name, arguments, 1)?;
    expect_max_outputs(name, context, 1)?;
    context.check_cancelled()?;
    let class = integer_class_argument(name, &arguments[0])?;
    let value = match (class, maximum) {
        (IntegerClass::I8, false) => fixed_width_scalar(i8::MIN),
        (IntegerClass::I8, true) => fixed_width_scalar(i8::MAX),
        (IntegerClass::U8, false) => fixed_width_scalar(u8::MIN),
        (IntegerClass::U8, true) => fixed_width_scalar(u8::MAX),
        (IntegerClass::I16, false) => fixed_width_scalar(i16::MIN),
        (IntegerClass::I16, true) => fixed_width_scalar(i16::MAX),
        (IntegerClass::U16, false) => fixed_width_scalar(u16::MIN),
        (IntegerClass::U16, true) => fixed_width_scalar(u16::MAX),
        (IntegerClass::I32, false) => fixed_width_scalar(i32::MIN),
        (IntegerClass::I32, true) => fixed_width_scalar(i32::MAX),
        (IntegerClass::U32, false) => fixed_width_scalar(u32::MIN),
        (IntegerClass::U32, true) => fixed_width_scalar(u32::MAX),
        (IntegerClass::I64, false) => fixed_width_scalar(i64::MIN),
        (IntegerClass::I64, true) => fixed_width_scalar(i64::MAX),
        (IntegerClass::U64, false) => fixed_width_scalar(u64::MIN),
        (IntegerClass::U64, true) => fixed_width_scalar(u64::MAX),
    }?;
    context.check_cancelled()?;
    Ok(vec![value])
}

fn integer_class_argument(name: &str, argument: &Value) -> Result<IntegerClass, BuiltinError> {
    let code_units = match argument {
        Value::String(value) => {
            let element = value.as_scalar().ok_or_else(|| {
                type_error(
                    name,
                    1,
                    "integer class-name char row vector or non-missing string scalar",
                    argument,
                )
            })?;
            if element.is_missing() {
                return Err(type_error(
                    name,
                    1,
                    "integer class-name char row vector or non-missing string scalar",
                    argument,
                ));
            }
            element.code_units().to_vec()
        }
        Value::Array(ArrayData::Char(array))
            if array.shape().dimensions() == [0, 0]
                || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
        {
            array.as_slice().iter().map(|value| value.get()).collect()
        }
        _ => {
            return Err(type_error(
                name,
                1,
                "integer class-name char row vector or non-missing string scalar",
                argument,
            ));
        }
    };
    match code_units.as_slice() {
        [105, 110, 116, 56] => Ok(IntegerClass::I8),
        [117, 105, 110, 116, 56] => Ok(IntegerClass::U8),
        [105, 110, 116, 49, 54] => Ok(IntegerClass::I16),
        [117, 105, 110, 116, 49, 54] => Ok(IntegerClass::U16),
        [105, 110, 116, 51, 50] => Ok(IntegerClass::I32),
        [117, 105, 110, 116, 51, 50] => Ok(IntegerClass::U32),
        [105, 110, 116, 54, 52] => Ok(IntegerClass::I64),
        [117, 105, 110, 116, 54, 52] => Ok(IntegerClass::U64),
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input 1 to `{name}` must name one of the eight fixed-width integer classes"),
        )),
    }
}

fn fixed_width_scalar<T: IntegerElement>(value: T) -> Result<Value, BuiltinError> {
    let shape = Shape::new([1, 1]).map_err(|error| array_error(&error))?;
    DenseArray::from_vec(shape, vec![value])
        .map(IntegerArrayData::from_typed)
        .map(ArrayData::Integer)
        .map(Value::Array)
        .map_err(|error| array_error(&error))
}

fn complex_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    if !arguments
        .iter()
        .any(|value| matches!(value, Value::Array(ArrayData::Integer(_))))
    {
        return core_numeric::complex_builtin(arguments, context);
    }
    expect_argument_count_range("complex", arguments, 1, 2)?;
    expect_max_outputs("complex", context, 1)?;
    context.check_cancelled()?;
    let Value::Array(ArrayData::Integer(real)) = &arguments[0] else {
        return Err(type_error(
            "complex",
            1,
            "real fixed-width integer array",
            &arguments[0],
        ));
    };
    if arguments.len() == 1 {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "integer `complex` construction currently requires separate real and imaginary inputs",
        ));
    }
    let Value::Array(ArrayData::Integer(imaginary)) = &arguments[1] else {
        return Err(type_error(
            "complex",
            2,
            "real fixed-width integer array of the same class",
            &arguments[1],
        ));
    };
    if real.is_complex() || imaginary.is_complex() || real.class_name() != imaginary.class_name() {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "the two-input integer form of `complex` requires real integer inputs of the same class",
        ));
    }
    let output = match (real, imaginary) {
        (IntegerArrayData::I8(real), IntegerArrayData::I8(imaginary)) => {
            combine_integer_components(real, imaginary, context)
        }
        (IntegerArrayData::U8(real), IntegerArrayData::U8(imaginary)) => {
            combine_integer_components(real, imaginary, context)
        }
        (IntegerArrayData::I16(real), IntegerArrayData::I16(imaginary)) => {
            combine_integer_components(real, imaginary, context)
        }
        (IntegerArrayData::U16(real), IntegerArrayData::U16(imaginary)) => {
            combine_integer_components(real, imaginary, context)
        }
        (IntegerArrayData::I32(real), IntegerArrayData::I32(imaginary)) => {
            combine_integer_components(real, imaginary, context)
        }
        (IntegerArrayData::U32(real), IntegerArrayData::U32(imaginary)) => {
            combine_integer_components(real, imaginary, context)
        }
        (IntegerArrayData::I64(real), IntegerArrayData::I64(imaginary)) => {
            combine_integer_components(real, imaginary, context)
        }
        (IntegerArrayData::U64(real), IntegerArrayData::U64(imaginary)) => {
            combine_integer_components(real, imaginary, context)
        }
        _ => Err(BuiltinError::new(
            BuiltinErrorCategory::Type,
            "the two-input integer form of `complex` requires real integer inputs of the same class",
        )),
    }?;
    context.check_cancelled()?;
    Ok(vec![output])
}

fn combine_integer_components<T>(
    real: &DenseArray<T>,
    imaginary: &DenseArray<T>,
    context: &BuiltinContext<'_>,
) -> Result<Value, BuiltinError>
where
    T: IntegerElement + Copy + Default + PartialEq,
    ComplexInteger<T>: IntegerElement,
{
    let shape = if real.shape().dimensions() == imaginary.shape().dimensions() {
        real.shape().clone()
    } else if real.numel() == 1 {
        imaginary.shape().clone()
    } else if imaginary.numel() == 1 {
        real.shape().clone()
    } else {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "the two-input form of `complex` requires equal shapes or one scalar input",
        ));
    };
    let length = usize::try_from(shape.numel()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "the `complex` output length does not fit this host",
        )
    })?;
    let mut values = Vec::new();
    values.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`complex` cannot allocate storage for {length} elements"),
        )
    })?;
    let mut has_nonzero_imaginary = false;
    for index in 0..length {
        if index.is_multiple_of(4_096) {
            context.check_cancelled()?;
        }
        let real = real.as_slice()[if real.numel() == 1 { 0 } else { index }];
        let imaginary = imaginary.as_slice()[if imaginary.numel() == 1 { 0 } else { index }];
        has_nonzero_imaginary |= imaginary != T::default();
        values.push(ComplexInteger::new(real, imaginary));
    }
    let integer = if has_nonzero_imaginary {
        DenseArray::from_vec(shape, values)
            .map(IntegerArrayData::from_typed)
            .map_err(|error| array_error(&error))?
    } else {
        let real = values.into_iter().map(ComplexInteger::re).collect();
        DenseArray::from_vec(shape, real)
            .map(IntegerArrayData::from_typed)
            .map_err(|error| array_error(&error))?
    };
    Ok(Value::Array(ArrayData::Integer(integer)))
}

fn string_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("string", arguments, 1)?;
    expect_max_outputs("string", context, 1)?;
    context.check_cancelled()?;
    let output = match &arguments[0] {
        Value::String(value) => Value::String(value.clone()),
        Value::Array(ArrayData::Char(array))
            if array.shape().dimensions() == [0, 0]
                || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
        {
            let code_units: Vec<u16> = array.as_slice().iter().map(|value| value.get()).collect();
            Value::String(StringValue::scalar(StringElement::from_code_units(
                code_units,
            )))
        }
        value => {
            return Err(type_error(
                "string",
                1,
                "char row vector or string value",
                value,
            ));
        }
    };
    context.check_cancelled()?;
    Ok(vec![output])
}

fn strings_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_max_outputs("strings", context, 1)?;
    context.check_cancelled()?;
    let dimensions = match arguments {
        [] => vec![1, 1],
        [single] => {
            let extent = nonnegative_dimension("strings", 1, single)?;
            vec![extent, extent]
        }
        _ => arguments
            .iter()
            .enumerate()
            .map(|(index, value)| nonnegative_dimension("strings", index + 1, value))
            .collect::<Result<Vec<_>, _>>()?,
    };
    let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    let length = usize::try_from(shape.numel()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "the `strings` output length does not fit this host",
        )
    })?;
    let mut elements = Vec::new();
    elements.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`strings` cannot allocate storage for {length} elements"),
        )
    })?;
    while elements.len() < length {
        context.check_cancelled()?;
        let target = elements.len().saturating_add(4_096).min(length);
        elements.resize(target, StringElement::default());
    }
    let array = StringArray::from_elements(shape, elements).map_err(|error| array_error(&error))?;
    context.check_cancelled()?;
    Ok(vec![Value::String(StringValue::Array(array))])
}

fn nonnegative_dimension(name: &str, position: usize, value: &Value) -> Result<u64, BuiltinError> {
    if let Some(component) = exact_real_integer_scalar(value) {
        return match component {
            IntegerComponent::Signed(value) => u64::try_from(value),
            IntegerComponent::Unsigned(value) => u64::try_from(value),
        }
        .map_err(|_| nonnegative_dimension_error(name, position));
    }
    let Some(value) = value.as_real_number() else {
        return Err(type_error(
            name,
            position,
            "finite nonnegative integer size scalar",
            value,
        ));
    };
    if !value.is_finite()
        || value < 0.0
        || value.fract() != 0.0
        || value >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(nonnegative_dimension_error(name, position));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as u64)
}

fn nonnegative_dimension_error(name: &str, position: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("input {position} to `{name}` must be a finite nonnegative integer size scalar"),
    )
}

fn cell_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_max_outputs("cell", context, 1)?;
    context.check_cancelled()?;
    let dimensions = cell_dimensions(arguments)?;
    let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    let length = usize::try_from(shape.numel()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "the `cell` output length does not fit this host",
        )
    })?;
    let mut values = reserved_values("cell", length)?;
    let empty_double = Value::empty_double();
    for index in 0..length {
        check_copy_cancelled(context, index)?;
        values.push(context.language_copy(&empty_double)?);
    }
    context.check_cancelled()?;
    let array =
        CellArray::from_values(shape, values).map_err(|error| aggregate_error("cell", &error))?;
    Ok(vec![Value::Cell(array)])
}

fn cell_dimensions(arguments: &[Value]) -> Result<Vec<u64>, BuiltinError> {
    match arguments {
        [] => Ok(vec![0, 0]),
        [single] => {
            let extent = nonnegative_dimension("cell", 1, single)?;
            Ok(vec![extent, extent])
        }
        _ => arguments
            .iter()
            .enumerate()
            .map(|(index, value)| nonnegative_dimension("cell", index + 1, value))
            .collect(),
    }
}

fn struct_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_max_outputs("struct", context, 1)?;
    context.check_cancelled()?;
    if arguments.is_empty() {
        return make_empty_struct([1, 1]);
    }
    if arguments.len() == 1 {
        return if is_zero_by_zero_real_double(&arguments[0]) {
            make_empty_struct([0, 0])
        } else {
            Err(BuiltinError::new(
                BuiltinErrorCategory::Other,
                "the accepted `struct` constructor supports only `struct([])` or ordered name/value pairs",
            ))
        };
    }
    if !arguments.len().is_multiple_of(2) {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `struct` expects ordered name/value pairs but received {} inputs",
                arguments.len()
            ),
        ));
    }

    let pair_count = arguments.len() / 2;
    let mut fields = reserved_fields(pair_count)?;
    for (pair, arguments) in arguments.chunks_exact(2).enumerate() {
        context.check_cancelled()?;
        fields.push(struct_field_name(pair * 2 + 1, &arguments[0])?);
    }
    StructSchema::new(fields.clone()).map_err(|error| aggregate_error("struct", &error))?;

    let shape = infer_struct_shape(arguments)?;
    let record_count = usize::try_from(shape.numel()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "the `struct` output length does not fit this host",
        )
    })?;
    let mut columns = reserved_struct_columns(pair_count, record_count)?;
    for (field, field_value) in arguments.iter().skip(1).step_by(2).enumerate() {
        copy_struct_field(&mut columns[field], field_value, record_count, context)?;
    }
    context.check_cancelled()?;
    StructArray::from_columns(shape, fields, columns)
        .map(Value::Struct)
        .map(|value| vec![value])
        .map_err(|error| aggregate_error("struct", &error))
}

fn make_empty_struct(dimensions: [u64; 2]) -> BuiltinResult {
    let shape = Shape::new(dimensions).map_err(|error| array_error(&error))?;
    StructArray::empty(shape, Vec::new())
        .map(Value::Struct)
        .map(|value| vec![value])
        .map_err(|error| aggregate_error("struct", &error))
}

fn is_zero_by_zero_real_double(value: &Value) -> bool {
    matches!(
        value,
        Value::Array(ArrayData::F64(array)) if array.shape().dimensions() == [0, 0]
    )
}

fn struct_field_name(position: usize, value: &Value) -> Result<FieldName, BuiltinError> {
    let code_units = match value {
        Value::String(string) => {
            let element = string.as_scalar().ok_or_else(|| {
                type_error(
                    "struct",
                    position,
                    "scalar char row or non-missing string scalar field name",
                    value,
                )
            })?;
            if element.is_missing() {
                return Err(invalid_struct_field_name(position));
            }
            element.code_units()
        }
        Value::Array(ArrayData::Char(array))
            if array.shape().dimensions() == [0, 0]
                || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
        {
            return ascii_field_name(position, array.as_slice().iter().map(|value| value.get()));
        }
        _ => {
            return Err(type_error(
                "struct",
                position,
                "scalar char row or non-missing string scalar field name",
                value,
            ));
        }
    };
    ascii_field_name(position, code_units.iter().copied())
}

fn ascii_field_name(
    position: usize,
    code_units: impl IntoIterator<Item = u16>,
) -> Result<FieldName, BuiltinError> {
    let mut name = String::new();
    for code_unit in code_units {
        let byte = u8::try_from(code_unit).map_err(|_| invalid_struct_field_name(position))?;
        if !byte.is_ascii() {
            return Err(invalid_struct_field_name(position));
        }
        name.push(char::from(byte));
    }
    FieldName::new(name).map_err(|error| aggregate_error("struct", &error))
}

fn invalid_struct_field_name(position: usize) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("input {position} to `struct` is not a valid ASCII field name"),
    )
}

fn infer_struct_shape(arguments: &[Value]) -> Result<Shape, BuiltinError> {
    let mut inferred: Option<&Shape> = None;
    for value in arguments.iter().skip(1).step_by(2) {
        let Value::Cell(array) = value else {
            continue;
        };
        if array.numel() == 1 {
            continue;
        }
        if let Some(expected) = inferred {
            if array.shape().dimensions() != expected.dimensions() {
                return Err(BuiltinError::new(
                    BuiltinErrorCategory::Domain,
                    "non-scalar cell inputs to `struct` must have identical shapes",
                ));
            }
        } else {
            inferred = Some(array.shape());
        }
    }
    inferred.cloned().map_or_else(
        || Shape::new([1, 1]).map_err(|error| array_error(&error)),
        Ok,
    )
}

fn reserved_struct_columns(
    field_count: usize,
    record_count: usize,
) -> Result<Vec<Vec<Value>>, BuiltinError> {
    let mut columns = Vec::new();
    columns.try_reserve_exact(field_count).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`struct` cannot allocate {field_count} field columns"),
        )
    })?;
    for _ in 0..field_count {
        columns.push(reserved_values("struct", record_count)?);
    }
    Ok(columns)
}

fn copy_struct_field(
    column: &mut Vec<Value>,
    field_value: &Value,
    record_count: usize,
    context: &mut BuiltinContext<'_>,
) -> Result<(), BuiltinError> {
    match field_value {
        Value::Cell(array) if array.numel() == 1 => {
            let value = array.value_at_offset(0).ok_or_else(|| {
                BuiltinError::new(
                    BuiltinErrorCategory::Other,
                    "a scalar cell field input had no stored value",
                )
            })?;
            for record in 0..record_count {
                check_copy_cancelled(context, record)?;
                column.push(context.language_copy(value)?);
            }
        }
        Value::Cell(array) => {
            for (record, value) in array.values().iter().enumerate() {
                check_copy_cancelled(context, record)?;
                column.push(context.language_copy(value)?);
            }
        }
        value => {
            for record in 0..record_count {
                check_copy_cancelled(context, record)?;
                column.push(context.language_copy(value)?);
            }
        }
    }
    Ok(())
}

fn deal_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    context.check_cancelled()?;
    let requested_outputs = context.requested_outputs();
    if arguments.len() != 1 && arguments.len() != requested_outputs {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `deal` received {} inputs for {requested_outputs} requested outputs",
                arguments.len()
            ),
        ));
    }
    let mut outputs = reserved_values("deal", requested_outputs)?;
    if let [value] = arguments {
        for output in 0..requested_outputs {
            check_copy_cancelled(context, output)?;
            outputs.push(context.language_copy(value)?);
        }
    } else {
        for (output, value) in arguments.iter().enumerate() {
            check_copy_cancelled(context, output)?;
            outputs.push(context.language_copy(value)?);
        }
    }
    context.check_cancelled()?;
    Ok(outputs)
}

fn reserved_values(name: &str, length: usize) -> Result<Vec<Value>, BuiltinError> {
    let mut values = Vec::new();
    values.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`{name}` cannot allocate storage for {length} values"),
        )
    })?;
    Ok(values)
}

fn reserved_fields(length: usize) -> Result<Vec<FieldName>, BuiltinError> {
    let mut fields = Vec::new();
    fields.try_reserve_exact(length).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("`struct` cannot allocate storage for {length} fields"),
        )
    })?;
    Ok(fields)
}

fn check_copy_cancelled(context: &BuiltinContext<'_>, index: usize) -> Result<(), BuiltinError> {
    if index.is_multiple_of(4_096) {
        context.check_cancelled()
    } else {
        Ok(())
    }
}

fn aggregate_error(name: &str, error: &openmat_value::AggregateError) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Domain,
        format!("`{name}` construction failed: {error}"),
    )
}

fn missing_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("missing", arguments, 0)?;
    expect_max_outputs("missing", context, 1)?;
    context.check_cancelled()?;
    Ok(vec![Value::String(StringValue::missing())])
}

fn class_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("class", arguments, 1)?;
    expect_max_outputs("class", context, 1)?;
    if !arguments[0].class_name().starts_with("matlab.") {
        return Ok(vec![Value::from(arguments[0].class_name())]);
    }
    let code_units = arguments[0]
        .class_name()
        .encode_utf16()
        .map(CharCodeUnit::new)
        .collect::<Vec<_>>();
    let shape = Shape::new([1, code_units.len() as u64])
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    let array = DenseArray::from_vec(shape, code_units)
        .map_err(|error| BuiltinError::new(BuiltinErrorCategory::Other, error.to_string()))?;
    Ok(vec![Value::Array(ArrayData::Char(array))])
}

fn isa_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("isa", arguments, 2)?;
    expect_max_outputs("isa", context, 1)?;
    let matches = class_name_argument_matches(&arguments[1], arguments[0].class_name())
        .ok_or_else(|| type_error("isa", 2, "char row vector or string scalar", &arguments[1]))?;
    Ok(vec![Value::Logical(matches)])
}

fn class_name_argument_matches(argument: &Value, expected: &str) -> Option<bool> {
    let expected = expected.encode_utf16().collect::<Vec<_>>();
    match argument {
        Value::String(value) => value
            .as_scalar()
            .map(|value| !value.is_missing() && value.code_units() == expected),
        Value::Array(ArrayData::Char(array))
            if array.shape().dimensions() == [0, 0]
                || (array.shape().ndims() == 2 && array.shape().extent(0) == 1) =>
        {
            Some(
                array
                    .as_slice()
                    .iter()
                    .map(|value| value.get())
                    .eq(expected),
            )
        }
        Value::Array(
            ArrayData::F64(_)
            | ArrayData::ComplexF64(_)
            | ArrayData::Logical(_)
            | ArrayData::Integer(_)
            | ArrayData::Char(_)
            | ArrayData::F32(_)
            | ArrayData::ComplexF32(_),
        )
        | Value::Sparse(_)
        | Value::Nothing
        | Value::Logical(_)
        | Value::Double(_)
        | Value::Complex(_)
        | Value::Cell(_)
        | Value::Struct(_)
        | Value::Table(_)
        | Value::Object(_)
        | Value::ObjectArray(_)
        | Value::Graphics(_)
        | Value::GraphicsArray(_)
        | Value::Function(_) => None,
    }
}

fn size_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count_range("size", arguments, 1, 2)?;
    context.check_cancelled()?;
    let dimensions = arguments[0]
        .dimensions()
        .ok_or_else(|| type_error("size", 1, "value with shape semantics", &arguments[0]))?;

    if arguments.len() == 2 {
        if context.requested_outputs() > 1 {
            return Err(BuiltinError::new(
                BuiltinErrorCategory::ArgumentCount,
                "`size(value, dimension)` supports one output",
            ));
        }
        let dimension = positive_integer_dimension("size", 2, &arguments[1])?;
        let extent = usize::try_from(dimension - 1)
            .ok()
            .and_then(|index| dimensions.get(index))
            .copied()
            .unwrap_or(1);
        return Ok(vec![dimension_scalar(extent)]);
    }

    let requested_outputs = context.requested_outputs().max(1);
    if requested_outputs == 1 {
        return Ok(vec![dimension_vector(dimensions)?]);
    }

    let mut outputs = Vec::with_capacity(requested_outputs);
    for index in 0..requested_outputs {
        let extent = if index + 1 == requested_outputs {
            dimensions
                .get(index..)
                .unwrap_or_default()
                .iter()
                .try_fold(1_u64, |product, extent| product.checked_mul(*extent))
                .ok_or_else(|| {
                    BuiltinError::new(
                        BuiltinErrorCategory::Domain,
                        "size output dimension product overflowed",
                    )
                })?
        } else {
            dimensions.get(index).copied().unwrap_or(1)
        };
        outputs.push(dimension_scalar(extent));
    }
    Ok(outputs)
}

fn numel_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("numel", arguments, 1)?;
    expect_max_outputs("numel", context, 1)?;
    context.check_cancelled()?;
    let count = arguments[0]
        .numel()
        .ok_or_else(|| type_error("numel", 1, "value with shape semantics", &arguments[0]))?;
    Ok(vec![dimension_scalar(count)])
}

fn disp_builtin(arguments: &[Value], context: &mut BuiltinContext<'_>) -> BuiltinResult {
    expect_argument_count("disp", arguments, 1)?;
    expect_exact_outputs("disp", context, 0)?;
    context.check_cancelled()?;
    context.emit(OutputEvent::Display(arguments[0].clone()))?;
    Ok(Vec::new())
}

fn dimension_vector(dimensions: &[u64]) -> Result<Value, BuiltinError> {
    let column_count = u64::try_from(dimensions.len()).map_err(|_| {
        BuiltinError::new(
            BuiltinErrorCategory::Domain,
            "dimension count does not fit the runtime shape model",
        )
    })?;
    let shape = Shape::new([1, column_count]).map_err(|error| array_error(&error))?;
    #[allow(clippy::cast_precision_loss)]
    let data = dimensions
        .iter()
        .map(|dimension| *dimension as f64)
        .collect();
    let array = DenseArray::from_vec(shape, data).map_err(|error| array_error(&error))?;
    Ok(Value::Array(ArrayData::F64(array)))
}

#[allow(clippy::cast_precision_loss)]
fn dimension_scalar(value: u64) -> Value {
    Value::Double(value as f64)
}

pub(crate) fn array_error(error: &openmat_array::ArrayError) -> BuiltinError {
    BuiltinError::new(BuiltinErrorCategory::Domain, error.to_string())
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn positive_integer_dimension(
    name: &str,
    position: usize,
    value: &Value,
) -> Result<u64, BuiltinError> {
    if let Some(component) = exact_real_integer_scalar(value) {
        let dimension = match component {
            IntegerComponent::Signed(value) => u64::try_from(value).ok().filter(|value| *value > 0),
            IntegerComponent::Unsigned(value) => {
                u64::try_from(value).ok().filter(|value| *value > 0)
            }
        };
        return dimension.ok_or_else(|| {
            BuiltinError::new(
                BuiltinErrorCategory::Domain,
                format!("input {position} to `{name}` must be a positive integer dimension"),
            )
        });
    }
    let Some(value) = value.as_real_number() else {
        return Err(type_error(name, position, "positive integer scalar", value));
    };
    if !value.is_finite()
        || value < 1.0
        || value.fract() != 0.0
        || value >= U64_EXCLUSIVE_UPPER_BOUND
    {
        return Err(BuiltinError::new(
            BuiltinErrorCategory::Domain,
            format!("input {position} to `{name}` must be a positive integer dimension"),
        ));
    }
    Ok(value as u64)
}

pub(crate) fn exact_real_integer_scalar(value: &Value) -> Option<IntegerComponent> {
    let Value::Array(ArrayData::Integer(integer)) = value else {
        return None;
    };
    if integer.is_complex() || integer.numel() != 1 {
        return None;
    }
    integer
        .element(0)
        .map(openmat_array::IntegerElementValue::real_component)
}

pub(crate) fn expect_argument_count(
    name: &str,
    arguments: &[Value],
    expected: usize,
) -> Result<(), BuiltinError> {
    if arguments.len() == expected {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `{name}` expects {expected} inputs but received {}",
                arguments.len()
            ),
        ))
    }
}

pub(crate) fn expect_argument_count_range(
    name: &str,
    arguments: &[Value],
    minimum: usize,
    maximum: usize,
) -> Result<(), BuiltinError> {
    if (minimum..=maximum).contains(&arguments.len()) {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `{name}` expects between {minimum} and {maximum} inputs but received {}",
                arguments.len()
            ),
        ))
    }
}

pub(crate) fn expect_max_outputs(
    name: &str,
    context: &BuiltinContext<'_>,
    maximum: usize,
) -> Result<(), BuiltinError> {
    if context.requested_outputs() <= maximum {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `{name}` supports at most {maximum} outputs but {} were requested",
                context.requested_outputs()
            ),
        ))
    }
}

fn expect_exact_outputs(
    name: &str,
    context: &BuiltinContext<'_>,
    expected: usize,
) -> Result<(), BuiltinError> {
    if context.requested_outputs() == expected {
        Ok(())
    } else {
        Err(BuiltinError::new(
            BuiltinErrorCategory::ArgumentCount,
            format!(
                "built-in `{name}` expects {expected} outputs but {} were requested",
                context.requested_outputs()
            ),
        ))
    }
}

pub(crate) fn type_error(
    name: &str,
    position: usize,
    expected: &str,
    actual: &Value,
) -> BuiltinError {
    BuiltinError::new(
        BuiltinErrorCategory::Type,
        format!(
            "input {position} to `{name}` must be a {expected}, found {}",
            actual.kind()
        ),
    )
}

#[cfg(test)]
mod tests {
    use openmat_array::{CharCodeUnit, Complex64 as ArrayComplex64, Logical};
    use openmat_runtime::{BuiltinContext, CancellationToken, OutputEvent, VecOutput};
    use openmat_value::{Complex64, StringArray, StringElement, StringValue};

    use super::*;

    fn invoke(
        registry: &BuiltinRegistry,
        name: &str,
        arguments: &[Value],
        output: &mut VecOutput,
    ) -> BuiltinResult {
        let cancellation = CancellationToken::new();
        invoke_with(registry, name, arguments, 1, &cancellation, output)
    }

    fn invoke_with(
        registry: &BuiltinRegistry,
        name: &str,
        arguments: &[Value],
        requested_outputs: usize,
        cancellation: &CancellationToken,
        output: &mut VecOutput,
    ) -> BuiltinResult {
        let handle = registry
            .handle_by_name(name)
            .expect("minimal built-in must be registered");
        let mut context = BuiltinContext::new(requested_outputs, cancellation, output);
        registry
            .invoke(handle, arguments, &mut context)
            .map_err(|error| match error {
                openmat_runtime::BuiltinInvocationError::Failed { error, .. } => error,
                openmat_runtime::BuiltinInvocationError::UnknownHandle(_) => {
                    panic!("looked-up built-in handle must resolve")
                }
            })
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn registers_only_the_documented_minimal_set() {
        let registry = minimal_registry().expect("fresh registry should accept built-ins");
        assert_eq!(registry.len(), 455);
        for name in [
            "__openmat_validate_argument_size",
            "__openmat_order_argument_fields",
            "__openmat_argument_isfield",
            "isfield",
            "mustBeNumeric",
            "mustBeFloat",
            "mustBeReal",
            "mustBeFinite",
            "mustBeNonNan",
            "mustBeNonempty",
            "mustBePositive",
            "mustBeNonnegative",
            "mustBeNegative",
            "mustBeNonpositive",
            "mustBeNonzero",
            "mustBeInteger",
            "mustBeMember",
            "pi",
            "i",
            "j",
            "true",
            "false",
            "eps",
            "Inf",
            "inf",
            "NaN",
            "nan",
            "realmin",
            "realmax",
            "flintmax",
            "expm1",
            "log1p",
            "angle",
            "class",
            "isa",
            "size",
            "numel",
            "ndims",
            "length",
            "isempty",
            "isnan",
            "isinf",
            "isfinite",
            "isreal",
            "isnumeric",
            "isfloat",
            "isinteger",
            "islogical",
            "ischar",
            "isstring",
            "iscell",
            "isstruct",
            "isobject",
            "isscalar",
            "isvector",
            "ismatrix",
            "isrow",
            "iscolumn",
            "isequal",
            "isequaln",
            "zeros",
            "ones",
            "eye",
            "sparse",
            "full",
            "issparse",
            "nnz",
            "nonzeros",
            "spones",
            "speye",
            "spalloc",
            "rand",
            "randn",
            "randi",
            "randperm",
            "rng",
            "reshape",
            "squeeze",
            "permute",
            "repmat",
            "logical",
            "double",
            "single",
            "char",
            "int8",
            "uint8",
            "int16",
            "uint16",
            "int32",
            "uint32",
            "int64",
            "uint64",
            "intmin",
            "intmax",
            "real",
            "imag",
            "conj",
            "complex",
            "string",
            "strings",
            "cell",
            "struct",
            "table",
            "array2table",
            "cell2table",
            "struct2table",
            "table2array",
            "table2cell",
            "table2struct",
            "height",
            "width",
            "istable",
            "head",
            "tail",
            "sortrows",
            "ismissing",
            "rmmissing",
            "fillmissing",
            "innerjoin",
            "outerjoin",
            "groupcounts",
            "groupsummary",
            "deal",
            "missing",
            "sum",
            "prod",
            "diff",
            "cumsum",
            "cumprod",
            "min",
            "max",
            "dot",
            "norm",
            "any",
            "all",
            "find",
            "diag",
            "triu",
            "tril",
            "trace",
            "cross",
            "kron",
            "meshgrid",
            "isosurface",
            "ndgrid",
            "flip",
            "fliplr",
            "flipud",
            "circshift",
            "unique",
            "ismember",
            "ipermute",
            "rot90",
            "shiftdim",
            "repelem",
            "blkdiag",
            "cummin",
            "cummax",
            "movsum",
            "movmean",
            "movmin",
            "movmax",
            "fix",
            "floor",
            "ceil",
            "exp",
            "log",
            "log10",
            "sin",
            "cos",
            "tan",
            "sign",
            "mod",
            "rem",
            "hypot",
            "asin",
            "acos",
            "atan",
            "atan2",
            "sinh",
            "cosh",
            "tanh",
            "asinh",
            "acosh",
            "atanh",
            "deg2rad",
            "rad2deg",
            "bitand",
            "bitor",
            "bitxor",
            "bitshift",
            "bitget",
            "bitset",
            "round",
            "linspace",
            "logspace",
            "quadgk",
            "mean",
            "median",
            "mode",
            "range",
            "bounds",
            "std",
            "var",
            "cov",
            "corrcoef",
            "sort",
            "cat",
            "horzcat",
            "vertcat",
            "conv",
            "conv2",
            "deconv",
            "poly",
            "polyval",
            "polyder",
            "polyint",
            "roots",
            "interp1",
            "lu",
            "qr",
            "chol",
            "det",
            "inv",
            "pinv",
            "sqrtm",
            "fft",
            "ifft",
            "fft2",
            "ifft2",
            "fftn",
            "ifftn",
            "fftshift",
            "ifftshift",
            "cart2pol",
            "pol2cart",
            "cart2sph",
            "sph2cart",
            "unwrap",
            "strcmp",
            "strcmpi",
            "strncmp",
            "strncmpi",
            "strlength",
            "upper",
            "lower",
            "reverse",
            "strip",
            "strtrim",
            "deblank",
            "contains",
            "startsWith",
            "endsWith",
            "strfind",
            "strrep",
            "erase",
            "replace",
            "sprintf",
            "num2str",
            "str2double",
            "regexp",
            "regexpi",
            "regexprep",
            "error",
            "assert",
            "warning",
            "lastwarn",
            "abs",
            "sqrt",
            "parula",
            "turbo",
            "hsv",
            "hot",
            "cool",
            "spring",
            "summer",
            "autumn",
            "winter",
            "gray",
            "bone",
            "copper",
            "pink",
            "jet",
            "lines",
            "colorcube",
            "prism",
            "flag",
            "white",
            "vga",
            "disp",
            "clc",
            "cd",
            "pwd",
            "format",
            "who",
            "whos",
            "fopen",
            "fclose",
            "fread",
            "fwrite",
            "fseek",
            "ftell",
            "textscan",
            "load",
            "save",
            "fileread",
            "readmatrix",
            "writematrix",
            "readtable",
            "writetable",
            "dir",
            "exist",
            "isfile",
            "isfolder",
            "mkdir",
            "rmdir",
            "delete",
            "copyfile",
            "movefile",
            "fullfile",
            "fileparts",
            "which",
            "path",
            "addpath",
            "rmpath",
            "genpath",
            "isvalid",
            "figure",
            "axes",
            "polaraxes",
            "subplot",
            "tiledlayout",
            "nexttile",
            "gcf",
            "gca",
            "groot",
            "plot",
            "polarplot",
            "stairs",
            "stem",
            "errorbar",
            "area",
            "bar",
            "histogram",
            "contour",
            "contourf",
            "image",
            "imagesc",
            "semilogx",
            "semilogy",
            "loglog",
            "scatter",
            "surf",
            "mesh",
            "patch",
            "fill",
            "fill3",
            "trisurf",
            "trimesh",
            "shading",
            "camlight",
            "lighting",
            "isonormals",
            "colorbar",
            "colormap",
            "view",
            "hold",
            "ishold",
            "isgraphics",
            "get",
            "set",
            "title",
            "xlabel",
            "ylabel",
            "zlabel",
            "legend",
            "xlim",
            "ylim",
            "zlim",
            "thetalim",
            "rlim",
            "clim",
            "caxis",
            "xticks",
            "xticklabels",
            "yticks",
            "yticklabels",
            "zticks",
            "zticklabels",
            "thetaticks",
            "rticks",
            "thetaticklabels",
            "rticklabels",
            "grid",
            "box",
            "cla",
            "clf",
            "close",
        ] {
            assert!(registry.handle_by_name(name).is_some());
        }
    }

    #[test]
    fn class_and_isa_report_basic_scalar_classes() {
        let registry = minimal_registry().expect("fresh registry should accept built-ins");
        let mut output = VecOutput::new();
        assert_eq!(
            invoke(
                &registry,
                "class",
                &[Value::Complex(Complex64::new(1.0, 2.0))],
                &mut output
            ),
            Ok(vec![Value::from("double")])
        );
        assert_eq!(
            invoke(
                &registry,
                "isa",
                &[Value::Logical(true), Value::from("logical")],
                &mut output
            ),
            Ok(vec![Value::Logical(true)])
        );

        let complex_array = Value::Array(ArrayData::ComplexF64(
            DenseArray::from_vec(
                Shape::new([1, 2]).unwrap(),
                vec![ArrayComplex64::new(1.0, 2.0), ArrayComplex64::ZERO],
            )
            .unwrap(),
        ));
        let logical_array = Value::Array(ArrayData::Logical(
            DenseArray::from_vec(
                Shape::new([2, 1]).unwrap(),
                vec![Logical::TRUE, Logical::FALSE],
            )
            .unwrap(),
        ));
        assert_eq!(
            invoke(&registry, "class", &[complex_array], &mut output),
            Ok(vec![Value::from("double")])
        );
        assert_eq!(
            invoke(
                &registry,
                "isa",
                &[logical_array, Value::from("logical")],
                &mut output
            ),
            Ok(vec![Value::Logical(true)])
        );
    }

    #[test]
    fn size_and_numel_cover_empty_and_higher_dimensional_arrays() {
        let registry = minimal_registry().expect("fresh registry should accept built-ins");
        let mut output = VecOutput::new();
        let empty = Value::Array(ArrayData::F64(
            DenseArray::from_vec(Shape::new([0, 3]).unwrap(), Vec::new()).unwrap(),
        ));
        let size = invoke(&registry, "size", std::slice::from_ref(&empty), &mut output).unwrap();
        let Value::Array(ArrayData::F64(size)) = &size[0] else {
            panic!("size must return a real row vector");
        };
        assert_eq!(size.shape().dimensions(), &[1, 2]);
        assert_eq!(size.as_slice(), &[0.0, 3.0]);
        assert_eq!(
            invoke(
                &registry,
                "class",
                std::slice::from_ref(&empty),
                &mut output,
            ),
            Ok(vec![Value::from("double")])
        );
        assert_eq!(
            invoke(
                &registry,
                "numel",
                std::slice::from_ref(&empty),
                &mut output,
            ),
            Ok(vec![Value::Double(0.0)])
        );

        let one_by_one = Value::Array(ArrayData::Logical(
            DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![Logical::TRUE]).unwrap(),
        ));
        let scalar_size = invoke(
            &registry,
            "size",
            std::slice::from_ref(&one_by_one),
            &mut output,
        )
        .unwrap();
        let Value::Array(ArrayData::F64(scalar_size)) = &scalar_size[0] else {
            panic!("size must return a real row vector");
        };
        assert_eq!(scalar_size.as_slice(), &[1.0, 1.0]);
        assert_eq!(
            invoke(&registry, "numel", &[one_by_one], &mut output),
            Ok(vec![Value::Double(1.0)])
        );

        let higher = Value::Array(ArrayData::ComplexF64(
            DenseArray::from_vec(
                Shape::new([2, 1, 3]).unwrap(),
                vec![ArrayComplex64::ZERO; 6],
            )
            .unwrap(),
        ));
        let cancellation = CancellationToken::new();
        assert_eq!(
            invoke_with(
                &registry,
                "size",
                std::slice::from_ref(&higher),
                2,
                &cancellation,
                &mut output,
            ),
            Ok(vec![Value::Double(2.0), Value::Double(3.0)])
        );
        assert_eq!(
            invoke(
                &registry,
                "size",
                &[higher, Value::Double(4.0)],
                &mut output,
            ),
            Ok(vec![Value::Double(1.0)])
        );
    }

    #[test]
    fn numeric_builtins_cover_real_and_complex_scalars() {
        let registry = minimal_registry().expect("fresh registry should accept built-ins");
        let mut output = VecOutput::new();
        assert_eq!(
            invoke(
                &registry,
                "abs",
                &[Value::Complex(Complex64::new(3.0, 4.0))],
                &mut output
            ),
            Ok(vec![Value::Double(5.0)])
        );
        assert_eq!(
            invoke(&registry, "sqrt", &[Value::Double(-4.0)], &mut output),
            Ok(vec![Value::Complex(Complex64::new(0.0, 2.0))])
        );
    }

    #[test]
    fn disp_emits_value_without_formatting_it() {
        let registry = minimal_registry().expect("fresh registry should accept built-ins");
        let mut output = VecOutput::new();
        let cancellation = CancellationToken::new();
        assert_eq!(
            invoke_with(
                &registry,
                "disp",
                &[Value::Double(42.0)],
                0,
                &cancellation,
                &mut output,
            ),
            Ok(Vec::new())
        );
        assert_eq!(
            output.events(),
            &[OutputEvent::Display(Value::Double(42.0))]
        );
    }

    #[test]
    fn disp_preserves_array_storage_and_observes_cancellation() {
        let registry = minimal_registry().expect("fresh registry should accept built-ins");
        let array = Value::Array(ArrayData::Logical(
            DenseArray::from_vec(
                Shape::new([1, 2]).unwrap(),
                vec![Logical::TRUE, Logical::FALSE],
            )
            .unwrap(),
        ));
        let mut output = VecOutput::new();
        let cancellation = CancellationToken::new();
        assert_eq!(
            invoke_with(
                &registry,
                "disp",
                std::slice::from_ref(&array),
                0,
                &cancellation,
                &mut output,
            ),
            Ok(Vec::new())
        );
        let OutputEvent::Display(displayed) = &output.events()[0] else {
            panic!("disp must emit an unnamed display event");
        };
        assert!(array.shares_array_storage_with(displayed));

        cancellation.cancel();
        let error =
            invoke_with(&registry, "disp", &[array], 0, &cancellation, &mut output).unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Cancelled);
        assert_eq!(output.events().len(), 1);
    }

    #[test]
    fn disp_emits_exact_utf16_string_elements_without_lossy_conversion() {
        let registry = minimal_registry().expect("fresh registry should accept built-ins");
        let string = Value::String(StringValue::Array(
            StringArray::from_elements(
                Shape::new([1, 2]).unwrap(),
                vec![
                    StringElement::from_code_units(vec![0xd83d]),
                    StringElement::missing(),
                ],
            )
            .unwrap(),
        ));
        let mut output = VecOutput::new();
        let cancellation = CancellationToken::new();
        assert_eq!(
            invoke_with(
                &registry,
                "disp",
                std::slice::from_ref(&string),
                0,
                &cancellation,
                &mut output,
            ),
            Ok(Vec::new())
        );
        let OutputEvent::Display(displayed) = &output.events()[0] else {
            panic!("disp must emit an unnamed display event");
        };
        assert!(string.shares_array_storage_with(displayed));
        let displayed = displayed.as_string_array().unwrap();
        assert_eq!(displayed.as_slice()[0].code_units(), &[0xd83d]);
        assert!(displayed.as_slice()[1].is_missing());
    }

    fn char_row(value: &str) -> Value {
        let code_units = value
            .encode_utf16()
            .map(CharCodeUnit::new)
            .collect::<Vec<_>>();
        let columns = u64::try_from(code_units.len()).unwrap();
        Value::Array(ArrayData::Char(
            DenseArray::from_vec(Shape::new([1, columns]).unwrap(), code_units).unwrap(),
        ))
    }

    fn integer_array<T: IntegerElement>(dimensions: [u64; 2], values: Vec<T>) -> Value {
        Value::Array(ArrayData::Integer(IntegerArrayData::from_typed(
            DenseArray::from_vec(Shape::new(dimensions).unwrap(), values).unwrap(),
        )))
    }

    #[test]
    fn integer_limits_cover_all_classes_without_floating_point() {
        let registry = minimal_registry().unwrap();
        let mut output = VecOutput::new();
        let cases = [
            ("int8", i8::MIN.to_string(), i8::MAX.to_string()),
            ("uint8", u8::MIN.to_string(), u8::MAX.to_string()),
            ("int16", i16::MIN.to_string(), i16::MAX.to_string()),
            ("uint16", u16::MIN.to_string(), u16::MAX.to_string()),
            ("int32", i32::MIN.to_string(), i32::MAX.to_string()),
            ("uint32", u32::MIN.to_string(), u32::MAX.to_string()),
            ("int64", i64::MIN.to_string(), i64::MAX.to_string()),
            ("uint64", u64::MIN.to_string(), u64::MAX.to_string()),
        ];
        for (class, minimum, maximum) in cases {
            let minimum_value =
                invoke(&registry, "intmin", &[char_row(class)], &mut output).unwrap();
            let maximum_value =
                invoke(&registry, "intmax", &[Value::from(class)], &mut output).unwrap();
            for (value, expected) in [(&minimum_value[0], minimum), (&maximum_value[0], maximum)] {
                let Value::Array(ArrayData::Integer(integer)) = value else {
                    panic!("integer limit must use exact integer storage");
                };
                assert_eq!(integer.class_name(), class);
                assert_eq!(integer.shape().dimensions(), &[1, 1]);
                assert!(!integer.is_complex());
                assert_eq!(
                    integer.element_decimal(0).unwrap().real_component(),
                    expected
                );
            }
        }

        let missing_error = invoke(
            &registry,
            "intmax",
            &[Value::String(StringValue::missing())],
            &mut output,
        )
        .unwrap_err();
        assert_eq!(missing_error.category, BuiltinErrorCategory::Type);
        let unknown_error =
            invoke(&registry, "intmax", &[Value::from("double")], &mut output).unwrap_err();
        assert_eq!(unknown_error.category, BuiltinErrorCategory::Domain);
        let matrix_error = invoke(
            &registry,
            "intmin",
            &[Value::Array(ArrayData::Char(
                DenseArray::from_vec(
                    Shape::new([2, 2]).unwrap(),
                    vec![
                        CharCodeUnit::new(105),
                        CharCodeUnit::new(110),
                        CharCodeUnit::new(116),
                        CharCodeUnit::new(56),
                    ],
                )
                .unwrap(),
            ))],
            &mut output,
        )
        .unwrap_err();
        assert_eq!(matrix_error.category, BuiltinErrorCategory::Type);
    }

    fn assert_complex_integer<T>(
        registry: &BuiltinRegistry,
        class: &str,
        real: Vec<T>,
        imaginary: Vec<T>,
        expected: &[(&str, &str)],
    ) where
        T: IntegerElement,
    {
        let mut output = VecOutput::new();
        let result = invoke(
            registry,
            "complex",
            &[
                integer_array([1, 2], real),
                integer_array([1, 2], imaginary),
            ],
            &mut output,
        )
        .unwrap();
        let Value::Array(ArrayData::Integer(integer)) = &result[0] else {
            panic!("integer complex constructor must preserve integer storage");
        };
        assert_eq!(integer.class_name(), class);
        assert_eq!(integer.shape().dimensions(), &[1, 2]);
        assert!(integer.is_complex());
        let components = integer
            .elements()
            .map(|value| {
                let decimal = value.canonical_decimal();
                (
                    decimal.real_component().to_owned(),
                    decimal.imaginary_component().to_owned(),
                )
            })
            .collect::<Vec<_>>();
        let expected = expected
            .iter()
            .map(|(real, imaginary)| ((*real).to_owned(), (*imaginary).to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(components, expected);
    }

    #[test]
    fn complex_preserves_all_fixed_width_integer_classes_and_existing_double_semantics() {
        let registry = minimal_registry().unwrap();
        assert_complex_integer(
            &registry,
            "int8",
            vec![i8::MIN, 7],
            vec![1, -9],
            &[("-128", "1"), ("7", "-9")],
        );
        assert_complex_integer(
            &registry,
            "uint8",
            vec![u8::MIN, u8::MAX],
            vec![1, 9],
            &[("0", "1"), ("255", "9")],
        );
        assert_complex_integer(
            &registry,
            "int16",
            vec![i16::MIN, i16::MAX],
            vec![1, -9],
            &[("-32768", "1"), ("32767", "-9")],
        );
        assert_complex_integer(
            &registry,
            "uint16",
            vec![u16::MIN, u16::MAX],
            vec![1, 9],
            &[("0", "1"), ("65535", "9")],
        );
        assert_complex_integer(
            &registry,
            "int32",
            vec![i32::MIN, i32::MAX],
            vec![1, -9],
            &[("-2147483648", "1"), ("2147483647", "-9")],
        );
        assert_complex_integer(
            &registry,
            "uint32",
            vec![u32::MIN, u32::MAX],
            vec![1, 9],
            &[("0", "1"), ("4294967295", "9")],
        );
        assert_complex_integer(
            &registry,
            "int64",
            vec![i64::MIN, i64::MAX],
            vec![1, -9],
            &[("-9223372036854775808", "1"), ("9223372036854775807", "-9")],
        );
        assert_complex_integer(
            &registry,
            "uint64",
            vec![u64::MIN, u64::MAX],
            vec![1, 9],
            &[("0", "1"), ("18446744073709551615", "9")],
        );

        let mut output = VecOutput::new();
        assert_eq!(
            invoke(
                &registry,
                "complex",
                &[Value::Double(3.0), Value::Double(-4.0)],
                &mut output,
            ),
            Ok(vec![Value::Complex(Complex64::new(3.0, -4.0))])
        );
    }

    #[test]
    fn complex_integer_shape_canonicalization_and_mismatches_are_stable() {
        let registry = minimal_registry().unwrap();
        let mut output = VecOutput::new();
        let broadcast = invoke(
            &registry,
            "complex",
            &[
                integer_array([1, 1], vec![7_i16]),
                integer_array([2, 1], vec![1_i16, -9]),
            ],
            &mut output,
        )
        .unwrap();
        let Value::Array(ArrayData::Integer(integer)) = &broadcast[0] else {
            panic!("broadcast integer complex result");
        };
        assert_eq!(integer.shape().dimensions(), &[2, 1]);
        assert_eq!(
            integer
                .elements()
                .map(|value| value.canonical_decimal().into_components())
                .collect::<Vec<_>>(),
            vec![
                ("7".to_owned(), "1".to_owned()),
                ("7".to_owned(), "-9".to_owned())
            ]
        );

        let canonical_real = invoke(
            &registry,
            "complex",
            &[
                integer_array([1, 1], vec![u64::MAX]),
                integer_array([1, 1], vec![0_u64]),
            ],
            &mut output,
        )
        .unwrap();
        let Value::Array(ArrayData::Integer(integer)) = &canonical_real[0] else {
            panic!("zero-imaginary integer result");
        };
        assert!(!integer.is_complex());
        assert_eq!(
            integer.element_decimal(0).unwrap().real_component(),
            u64::MAX.to_string()
        );

        let signed_negative_for_unsigned = invoke(
            &registry,
            "complex",
            &[
                integer_array([1, 1], vec![1_u8]),
                integer_array([1, 1], vec![-1_i8]),
            ],
            &mut output,
        )
        .unwrap_err();
        assert_eq!(
            signed_negative_for_unsigned.category,
            BuiltinErrorCategory::Type
        );

        let shape_error = invoke(
            &registry,
            "complex",
            &[
                integer_array([1, 2], vec![1_i8, 2]),
                integer_array([1, 3], vec![1_i8, 2, 3]),
            ],
            &mut output,
        )
        .unwrap_err();
        assert_eq!(shape_error.category, BuiltinErrorCategory::Domain);

        let type_error = invoke(
            &registry,
            "complex",
            &[integer_array([1, 1], vec![1_i8]), Value::Double(2.0)],
            &mut output,
        )
        .unwrap_err();
        assert_eq!(type_error.category, BuiltinErrorCategory::Type);

        let already_complex = Value::Array(ArrayData::Integer(IntegerArrayData::ComplexI8(
            DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![ComplexInteger::new(1, 2)])
                .unwrap(),
        )));
        let complex_input_error = invoke(
            &registry,
            "complex",
            &[already_complex, integer_array([1, 1], vec![1_i8])],
            &mut output,
        )
        .unwrap_err();
        assert_eq!(complex_input_error.category, BuiltinErrorCategory::Type);

        let one_input_error = invoke(
            &registry,
            "complex",
            &[integer_array([1, 1], vec![1_i8])],
            &mut output,
        )
        .unwrap_err();
        assert_eq!(one_input_error.category, BuiltinErrorCategory::Type);
    }

    #[test]
    fn complex_integer_preserves_multidimensional_column_major_order() {
        let registry = minimal_registry().unwrap();
        let mut output = VecOutput::new();
        let result = invoke(
            &registry,
            "complex",
            &[
                integer_array([2, 2], vec![1_i32, 2, 3, 4]),
                integer_array([2, 2], vec![-1_i32, -2, -3, -4]),
            ],
            &mut output,
        )
        .unwrap();
        let Value::Array(ArrayData::Integer(integer)) = &result[0] else {
            panic!("2-D integer complex result");
        };
        assert_eq!(integer.shape().dimensions(), &[2, 2]);
        assert_eq!(
            integer
                .elements()
                .map(|value| value.canonical_decimal().into_components())
                .collect::<Vec<_>>(),
            vec![
                ("1".to_owned(), "-1".to_owned()),
                ("2".to_owned(), "-2".to_owned()),
                ("3".to_owned(), "-3".to_owned()),
                ("4".to_owned(), "-4".to_owned()),
            ]
        );
    }

    #[test]
    fn string_construction_preserves_surrogate_empty_and_missing_states() {
        let registry = minimal_registry().unwrap();
        let mut output = VecOutput::new();
        let strings = invoke(
            &registry,
            "strings",
            &[Value::Double(1.0), Value::Double(3.0)],
            &mut output,
        )
        .unwrap();
        let array = strings[0].as_string_array().unwrap();
        assert_eq!(array.shape().dimensions(), &[1, 3]);
        assert!(
            array
                .as_slice()
                .iter()
                .all(|value| !value.is_missing() && value.code_units().is_empty())
        );

        let isolated_surrogate = Value::Array(ArrayData::Char(
            DenseArray::from_vec(Shape::new([1, 1]).unwrap(), vec![CharCodeUnit::new(0xd83d)])
                .unwrap(),
        ));
        let converted = invoke(&registry, "string", &[isolated_surrogate], &mut output).unwrap();
        let surrogate = converted[0].as_string_scalar().unwrap();
        assert!(!surrogate.is_missing());
        assert_eq!(surrogate.code_units(), &[0xd83d]);

        let empty = invoke(&registry, "string", &[Value::from("")], &mut output).unwrap();
        let missing = invoke(&registry, "missing", &[], &mut output).unwrap();
        let empty = empty[0].as_string_scalar().unwrap();
        let missing = missing[0].as_string_scalar().unwrap();
        assert!(empty.code_units().is_empty());
        assert!(!empty.is_missing());
        assert!(missing.code_units().is_empty());
        assert!(missing.is_missing());

        let empty_array = invoke(
            &registry,
            "strings",
            &[Value::Double(0.0), Value::Double(2.0)],
            &mut output,
        )
        .unwrap();
        assert_eq!(
            empty_array[0]
                .as_string_array()
                .unwrap()
                .shape()
                .dimensions(),
            &[0, 2]
        );
        let negative = invoke(
            &registry,
            "strings",
            &[Value::Double(-1.0), Value::Double(2.0)],
            &mut output,
        )
        .unwrap_err();
        assert_eq!(negative.category, BuiltinErrorCategory::Domain);
    }

    #[test]
    fn size_and_numel_report_structured_input_errors() {
        let registry = minimal_registry().expect("fresh registry should accept built-ins");
        let mut output = VecOutput::new();
        let error = invoke(
            &registry,
            "size",
            &[Value::Double(1.0), Value::Double(0.0)],
            &mut output,
        )
        .unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::Domain);

        let error = invoke(&registry, "numel", &[], &mut output).unwrap_err();
        assert_eq!(error.category, BuiltinErrorCategory::ArgumentCount);
    }
}
