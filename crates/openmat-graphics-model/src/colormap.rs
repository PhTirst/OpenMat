//! MATLAB R2022b-compatible scientific colormaps.

/// Default number of colors in MATLAB R2022b figures.
pub const DEFAULT_COLORMAP_LENGTH: usize = 256;

/// Exact MATLAB R2022b `parula(256)` behavior captured by the black-box oracle.
///
/// MATLAB R2022b derives every other requested length by linearly resampling this
/// table over its full endpoint-inclusive domain.
#[allow(clippy::approx_constant)]
pub const PARULA_R2022B: [[f64; 3]; DEFAULT_COLORMAP_LENGTH] = [
    [0.2422, 0.1504, 0.6603],
    [0.2444, 0.1534, 0.6728],
    [0.2464, 0.1569, 0.6847],
    [0.2484, 0.1607, 0.6961],
    [0.2503, 0.1648, 0.7071],
    [0.2522, 0.1689, 0.7179],
    [0.2540, 0.1732, 0.7286],
    [0.2558, 0.1773, 0.7393],
    [0.2576, 0.1814, 0.7501],
    [0.2594, 0.1854, 0.7610],
    [0.2611, 0.1893, 0.7719],
    [0.2628, 0.1932, 0.7828],
    [0.2645, 0.1972, 0.7937],
    [0.2661, 0.2011, 0.8043],
    [0.2676, 0.2052, 0.8148],
    [0.2691, 0.2094, 0.8249],
    [0.2704, 0.2138, 0.8346],
    [0.2717, 0.2184, 0.8439],
    [0.2729, 0.2231, 0.8528],
    [0.2740, 0.2280, 0.8612],
    [0.2749, 0.2330, 0.8692],
    [0.2758, 0.2382, 0.8767],
    [0.2766, 0.2435, 0.8840],
    [0.2774, 0.2489, 0.8908],
    [0.2781, 0.2543, 0.8973],
    [0.2788, 0.2598, 0.9035],
    [0.2794, 0.2653, 0.9094],
    [0.2798, 0.2708, 0.9150],
    [0.2802, 0.2764, 0.9204],
    [0.2806, 0.2819, 0.9255],
    [0.2809, 0.2875, 0.9305],
    [0.2811, 0.2930, 0.9352],
    [0.2813, 0.2985, 0.9397],
    [0.2814, 0.3040, 0.9441],
    [0.2814, 0.3095, 0.9483],
    [0.2813, 0.3150, 0.9524],
    [0.2811, 0.3204, 0.9563],
    [0.2809, 0.3259, 0.9600],
    [0.2807, 0.3313, 0.9636],
    [0.2803, 0.3367, 0.9670],
    [0.2798, 0.3421, 0.9702],
    [0.2791, 0.3475, 0.9733],
    [0.2784, 0.3529, 0.9763],
    [0.2776, 0.3583, 0.9791],
    [0.2766, 0.3638, 0.9817],
    [0.2754, 0.3693, 0.9840],
    [0.2741, 0.3748, 0.9862],
    [0.2726, 0.3804, 0.9881],
    [0.2710, 0.3860, 0.9898],
    [0.2691, 0.3916, 0.9912],
    [0.2670, 0.3973, 0.9924],
    [0.2647, 0.4030, 0.9935],
    [0.2621, 0.4088, 0.9946],
    [0.2591, 0.4145, 0.9955],
    [0.2556, 0.4203, 0.9965],
    [0.2517, 0.4261, 0.9974],
    [0.2473, 0.4319, 0.9983],
    [0.2424, 0.4378, 0.9991],
    [0.2369, 0.4437, 0.9996],
    [0.2311, 0.4497, 0.9995],
    [0.2250, 0.4559, 0.9985],
    [0.2189, 0.4620, 0.9968],
    [0.2128, 0.4682, 0.9948],
    [0.2066, 0.4743, 0.9926],
    [0.2006, 0.4803, 0.9906],
    [0.1950, 0.4861, 0.9887],
    [0.1903, 0.4919, 0.9867],
    [0.1869, 0.4975, 0.9844],
    [0.1847, 0.5030, 0.9819],
    [0.1831, 0.5084, 0.9793],
    [0.1818, 0.5138, 0.9766],
    [0.1806, 0.5191, 0.9738],
    [0.1795, 0.5244, 0.9709],
    [0.1785, 0.5296, 0.9677],
    [0.1778, 0.5349, 0.9641],
    [0.1773, 0.5401, 0.9602],
    [0.1768, 0.5452, 0.9560],
    [0.1764, 0.5504, 0.9516],
    [0.1755, 0.5554, 0.9473],
    [0.1740, 0.5605, 0.9432],
    [0.1716, 0.5655, 0.9393],
    [0.1686, 0.5705, 0.9357],
    [0.1649, 0.5755, 0.9323],
    [0.1610, 0.5805, 0.9289],
    [0.1573, 0.5854, 0.9254],
    [0.1540, 0.5902, 0.9218],
    [0.1513, 0.5950, 0.9182],
    [0.1492, 0.5997, 0.9147],
    [0.1475, 0.6043, 0.9113],
    [0.1461, 0.6089, 0.9080],
    [0.1446, 0.6135, 0.9050],
    [0.1429, 0.6180, 0.9022],
    [0.1408, 0.6226, 0.8998],
    [0.1383, 0.6272, 0.8975],
    [0.1354, 0.6317, 0.8953],
    [0.1321, 0.6363, 0.8932],
    [0.1288, 0.6408, 0.8910],
    [0.1253, 0.6453, 0.8887],
    [0.1219, 0.6497, 0.8862],
    [0.1185, 0.6541, 0.8834],
    [0.1152, 0.6584, 0.8804],
    [0.1119, 0.6627, 0.8770],
    [0.1085, 0.6669, 0.8734],
    [0.1048, 0.6710, 0.8695],
    [0.1009, 0.6750, 0.8653],
    [0.0964, 0.6789, 0.8609],
    [0.0914, 0.6828, 0.8562],
    [0.0855, 0.6865, 0.8513],
    [0.0789, 0.6902, 0.8462],
    [0.0713, 0.6938, 0.8409],
    [0.0628, 0.6972, 0.8355],
    [0.0535, 0.7006, 0.8299],
    [0.0433, 0.7039, 0.8242],
    [0.0328, 0.7071, 0.8183],
    [0.0234, 0.7103, 0.8124],
    [0.0155, 0.7133, 0.8064],
    [0.0091, 0.7163, 0.8003],
    [0.0046, 0.7192, 0.7941],
    [0.0019, 0.7220, 0.7878],
    [0.0009, 0.7248, 0.7815],
    [0.0018, 0.7275, 0.7752],
    [0.0046, 0.7301, 0.7688],
    [0.0094, 0.7327, 0.7623],
    [0.0162, 0.7352, 0.7558],
    [0.0253, 0.7376, 0.7492],
    [0.0369, 0.7400, 0.7426],
    [0.0504, 0.7423, 0.7359],
    [0.0638, 0.7446, 0.7292],
    [0.0770, 0.7468, 0.7224],
    [0.0899, 0.7489, 0.7156],
    [0.1023, 0.7510, 0.7088],
    [0.1141, 0.7531, 0.7019],
    [0.1252, 0.7552, 0.6950],
    [0.1354, 0.7572, 0.6881],
    [0.1448, 0.7593, 0.6812],
    [0.1532, 0.7614, 0.6741],
    [0.1609, 0.7635, 0.6671],
    [0.1678, 0.7656, 0.6599],
    [0.1741, 0.7678, 0.6527],
    [0.1799, 0.7699, 0.6454],
    [0.1853, 0.7721, 0.6379],
    [0.1905, 0.7743, 0.6303],
    [0.1954, 0.7765, 0.6225],
    [0.2003, 0.7787, 0.6146],
    [0.2061, 0.7808, 0.6065],
    [0.2118, 0.7828, 0.5983],
    [0.2178, 0.7849, 0.5899],
    [0.2244, 0.7869, 0.5813],
    [0.2318, 0.7887, 0.5725],
    [0.2401, 0.7905, 0.5636],
    [0.2491, 0.7922, 0.5546],
    [0.2589, 0.7937, 0.5454],
    [0.2695, 0.7951, 0.5360],
    [0.2809, 0.7964, 0.5266],
    [0.2929, 0.7975, 0.5170],
    [0.3052, 0.7985, 0.5074],
    [0.3176, 0.7994, 0.4975],
    [0.3301, 0.8002, 0.4876],
    [0.3424, 0.8009, 0.4774],
    [0.3548, 0.8016, 0.4669],
    [0.3671, 0.8021, 0.4563],
    [0.3795, 0.8026, 0.4454],
    [0.3921, 0.8029, 0.4344],
    [0.4050, 0.8031, 0.4233],
    [0.4184, 0.8030, 0.4122],
    [0.4322, 0.8028, 0.4013],
    [0.4463, 0.8024, 0.3904],
    [0.4608, 0.8018, 0.3797],
    [0.4753, 0.8011, 0.3691],
    [0.4899, 0.8002, 0.3586],
    [0.5044, 0.7993, 0.3480],
    [0.5187, 0.7982, 0.3374],
    [0.5329, 0.7970, 0.3267],
    [0.5470, 0.7957, 0.3159],
    [0.5609, 0.7943, 0.3050],
    [0.5748, 0.7929, 0.2941],
    [0.5886, 0.7913, 0.2833],
    [0.6024, 0.7896, 0.2726],
    [0.6161, 0.7878, 0.2622],
    [0.6297, 0.7859, 0.2521],
    [0.6433, 0.7839, 0.2423],
    [0.6567, 0.7818, 0.2329],
    [0.6701, 0.7796, 0.2239],
    [0.6833, 0.7773, 0.2155],
    [0.6963, 0.7750, 0.2075],
    [0.7091, 0.7727, 0.1998],
    [0.7218, 0.7703, 0.1924],
    [0.7344, 0.7679, 0.1852],
    [0.7468, 0.7654, 0.1782],
    [0.7590, 0.7629, 0.1717],
    [0.7710, 0.7604, 0.1658],
    [0.7829, 0.7579, 0.1608],
    [0.7945, 0.7554, 0.1570],
    [0.8060, 0.7529, 0.1546],
    [0.8172, 0.7505, 0.1535],
    [0.8281, 0.7481, 0.1536],
    [0.8389, 0.7457, 0.1546],
    [0.8495, 0.7435, 0.1564],
    [0.8600, 0.7413, 0.1587],
    [0.8703, 0.7392, 0.1615],
    [0.8804, 0.7372, 0.1650],
    [0.8903, 0.7353, 0.1695],
    [0.9000, 0.7336, 0.1749],
    [0.9093, 0.7321, 0.1815],
    [0.9184, 0.7308, 0.1890],
    [0.9272, 0.7298, 0.1973],
    [0.9357, 0.7290, 0.2061],
    [0.9440, 0.7285, 0.2151],
    [0.9523, 0.7284, 0.2237],
    [0.9606, 0.7285, 0.2312],
    [0.9689, 0.7292, 0.2373],
    [0.9770, 0.7304, 0.2418],
    [0.9842, 0.7330, 0.2446],
    [0.9900, 0.7365, 0.2429],
    [0.9946, 0.7407, 0.2394],
    [0.9966, 0.7458, 0.2351],
    [0.9971, 0.7513, 0.2309],
    [0.9972, 0.7569, 0.2267],
    [0.9971, 0.7626, 0.2224],
    [0.9969, 0.7683, 0.2181],
    [0.9966, 0.7740, 0.2138],
    [0.9962, 0.7798, 0.2095],
    [0.9957, 0.7856, 0.2053],
    [0.9949, 0.7915, 0.2012],
    [0.9938, 0.7974, 0.1974],
    [0.9923, 0.8034, 0.1939],
    [0.9906, 0.8095, 0.1906],
    [0.9885, 0.8156, 0.1875],
    [0.9861, 0.8218, 0.1846],
    [0.9835, 0.8280, 0.1817],
    [0.9807, 0.8342, 0.1787],
    [0.9778, 0.8404, 0.1757],
    [0.9748, 0.8467, 0.1726],
    [0.9720, 0.8529, 0.1695],
    [0.9694, 0.8591, 0.1665],
    [0.9671, 0.8654, 0.1636],
    [0.9651, 0.8716, 0.1608],
    [0.9634, 0.8778, 0.1582],
    [0.9619, 0.8840, 0.1557],
    [0.9608, 0.8902, 0.1532],
    [0.9601, 0.8963, 0.1507],
    [0.9596, 0.9023, 0.1480],
    [0.9595, 0.9084, 0.1450],
    [0.9597, 0.9143, 0.1418],
    [0.9601, 0.9203, 0.1382],
    [0.9608, 0.9262, 0.1344],
    [0.9618, 0.9320, 0.1304],
    [0.9629, 0.9379, 0.1261],
    [0.9642, 0.9437, 0.1216],
    [0.9657, 0.9494, 0.1168],
    [0.9674, 0.9552, 0.1116],
    [0.9692, 0.9609, 0.1061],
    [0.9711, 0.9667, 0.1001],
    [0.9730, 0.9724, 0.0938],
    [0.9749, 0.9782, 0.0872],
    [0.9769, 0.9839, 0.0805],
];

/// Exact MATLAB R2022b `turbo(256)` behavior captured by the black-box oracle.
///
/// Other requested lengths are linearly resampled over the endpoint-inclusive
/// domain, matching the R2022b implementation.
#[allow(clippy::unreadable_literal)]
pub const TURBO_R2022B: [[f64; 3]; DEFAULT_COLORMAP_LENGTH] = [
    [
        f64::from_bits(0x3fc8504816f0068e),
        f64::from_bits(0x3fb25edd052934ad),
        f64::from_bits(0x3fcdb7bf1e8e6080),
    ],
    [
        f64::from_bits(0x3fc8f0307f23cc8e),
        f64::from_bits(0x3fb5590c0ad03d9b),
        f64::from_bits(0x3fd0bc408d8ec95c),
    ],
    [
        f64::from_bits(0x3fc98b2e9ccb7d41),
        f64::from_bits(0x3fb8509bf9c62a1b),
        f64::from_bits(0x3fd2934acaff6d33),
    ],
    [
        f64::from_bits(0x3fca219652bd3c36),
        f64::from_bits(0x3fbb44e50c5eb314),
        f64::from_bits(0x3fd461522a6f3f53),
    ],
    [
        f64::from_bits(0x3fcab367a0f9096c),
        f64::from_bits(0x3fbe368f08461f9f),
        f64::from_bits(0x3fd62602c9081c2e),
    ],
    [
        f64::from_bits(0x3fcb40a2877ee4e2),
        f64::from_bits(0x3fc0927913e81451),
        f64::from_bits(0x3fd7e1869835158c),
    ],
    [
        f64::from_bits(0x3fcbc947064ece9a),
        f64::from_bits(0x3fc20807357e670e),
        f64::from_bits(0x3fd993b3a68b19a4),
    ],
    [
        f64::from_bits(0x3fcc4d551d68c693),
        f64::from_bits(0x3fc37c45cbbc2b95),
        f64::from_bits(0x3fdb3cb3e5753a3f),
    ],
    [
        f64::from_bits(0x3fcccccccccccccd),
        f64::from_bits(0x3fc4eee0f3cb3e57),
        f64::from_bits(0x3fdcdc8754f3775c),
    ],
    [
        f64::from_bits(0x3fcd47ae147ae148),
        f64::from_bits(0x3fc6602c9081c2e3),
        f64::from_bits(0x3fde732df505d0fa),
    ],
    [
        f64::from_bits(0x3fcdbdf8f4730404),
        f64::from_bits(0x3fc7cfd4bf0995ab),
        f64::from_bits(0x3fe00053e2d6238e),
    ],
    [
        f64::from_bits(0x3fce2f5989df1173),
        f64::from_bits(0x3fc93dd97f62b6ae),
        f64::from_bits(0x3fe0c2656abde3fc),
    ],
    [
        f64::from_bits(0x3fce9c779a6b50b1),
        f64::from_bits(0x3fcaaa8eb463497b),
        f64::from_bits(0x3fe17fe08aefb2ab),
    ],
    [
        f64::from_bits(0x3fcf04ff43419e30),
        f64::from_bits(0x3fcc15a07b352a84),
        f64::from_bits(0x3fe238b04ab606b8),
    ],
    [
        f64::from_bits(0x3fcf68f08461f9f0),
        f64::from_bits(0x3fcd7f0ed3d859c9),
        f64::from_bits(0x3fe2ece9a2c66905),
    ],
    [
        f64::from_bits(0x3fcfc84b5dcc63f1),
        f64::from_bits(0x3fcee72da122fad7),
        f64::from_bits(0x3fe39c8c9320d994),
    ],
    [
        f64::from_bits(0x3fd01187e7c06e1a),
        f64::from_bits(0x3fd026d4801f7510),
        f64::from_bits(0x3fe447991bc55864),
    ],
    [
        f64::from_bits(0x3fd03c74fb549f95),
        f64::from_bits(0x3fd0d96a6a01259a),
        f64::from_bits(0x3fe4ee0f3cb3e575),
    ],
    [
        f64::from_bits(0x3fd06540cc78e9f7),
        f64::from_bits(0x3fd18b2e9ccb7d41),
        f64::from_bits(0x3fe58fd9fd36f7e4),
    ],
    [
        f64::from_bits(0x3fd08bc169c23b79),
        f64::from_bits(0x3fd23c21187e7c07),
        f64::from_bits(0x3fe62d0e56041893),
    ],
    [
        f64::from_bits(0x3fd0aff6d330941d),
        f64::from_bits(0x3fd2ec6bce8533b1),
        f64::from_bits(0x3fe6c5974e65bea1),
    ],
    [
        f64::from_bits(0x3fd0d1b71758e219),
        f64::from_bits(0x3fd39be4cd749279),
        f64::from_bits(0x3fe75989df1172ef),
    ],
    [
        f64::from_bits(0x3fd0f156191148fe),
        f64::from_bits(0x3fd44ab606b7aa26),
        f64::from_bits(0x3fe7e8e60807357e),
    ],
    [
        f64::from_bits(0x3fd10ea9e6eeb702),
        f64::from_bits(0x3fd4f8b588e368f1),
        f64::from_bits(0x3fe873abc947064f),
    ],
    [
        f64::from_bits(0x3fd129888f861a61),
        f64::from_bits(0x3fd5a5e353f7ced9),
        f64::from_bits(0x3fe8f9db22d0e560),
    ],
    [
        f64::from_bits(0x3fd14245f5ad96a7),
        f64::from_bits(0x3fd65269595feda6),
        f64::from_bits(0x3fe97b5f1bef49cf),
    ],
    [
        f64::from_bits(0x3fd1588e368f0846),
        f64::from_bits(0x3fd6fe1da7b0b392),
        f64::from_bits(0x3fe9f84cad57bc7f),
    ],
    [
        f64::from_bits(0x3fd16cb5350092cd),
        f64::from_bits(0x3fd7a92a30553261),
        f64::from_bits(0x3fea708ede54b48d),
    ],
    [
        f64::from_bits(0x3fd17e670e2c12ae),
        f64::from_bits(0x3fd8533b10774688),
        f64::from_bits(0x3feae44fa05143bf),
    ],
    [
        f64::from_bits(0x3fd18df7a4e7ab75),
        f64::from_bits(0x3fd8fcce1c58255b),
        f64::from_bits(0x3feb536501e2584f),
    ],
    [
        f64::from_bits(0x3fd19b13165d3997),
        f64::from_bits(0x3fd9a5657fb69985),
        f64::from_bits(0x3febbdcf0307f23d),
    ],
    [
        f64::from_bits(0x3fd1a60d4562e0a0),
        f64::from_bits(0x3fda4d551d68c693),
        f64::from_bits(0x3fec23b7952d234f),
    ],
    [
        f64::from_bits(0x3fd1ae924f227d03),
        f64::from_bits(0x3fdaf49cf56eac86),
        f64::from_bits(0x3fec84f4c6e6d9be),
    ],
    [
        f64::from_bits(0x3fd1b4f61672324d),
        f64::from_bits(0x3fdb9b13165d3997),
        f64::from_bits(0x3fece19b90ea9e6f),
    ],
    [
        f64::from_bits(0x3fd1b8e4b87bdcf0),
        f64::from_bits(0x3fdc40b780346dc6),
        f64::from_bits(0x3fed3996fa82e87d),
    ],
    [
        f64::from_bits(0x3fd1ba8826aa8eb4),
        f64::from_bits(0x3fdce5b4245f5ad9),
        f64::from_bits(0x3fed8d10f51ac9b0),
    ],
    [
        f64::from_bits(0x3fd1ba0a52695960),
        f64::from_bits(0x3fdd89b52007dd44),
        f64::from_bits(0x3feddbdf8f473040),
    ],
    [
        f64::from_bits(0x3fd1b71758e21965),
        f64::from_bits(0x3fde2d38476f2a5a),
        f64::from_bits(0x3fee2602c9081c2e),
    ],
    [
        f64::from_bits(0x3fd1b1d92b7fe08b),
        f64::from_bits(0x3fdecfe9b7bf1e8e),
        f64::from_bits(0x3fee6ba493c89f41),
    ],
    [
        f64::from_bits(0x3fd1aa79bbadc098),
        f64::from_bits(0x3fdf71c970f7b9e0),
        f64::from_bits(0x3feeac9afe1da7b1),
    ],
    [
        f64::from_bits(0x3fd1a0a5269595ff),
        f64::from_bits(0x3fe0096bb98c7e28),
        f64::from_bits(0x3feee8fb00bcbe62),
    ],
    [
        f64::from_bits(0x3fd194855da27286),
        f64::from_bits(0x3fe0599ed7c6fbd2),
        f64::from_bits(0x3fef20c49ba5e354),
    ],
    [
        f64::from_bits(0x3fd1861a60d4562e),
        f64::from_bits(0x3fe0a97e132b55ef),
        f64::from_bits(0x3fef53e2d6238da4),
    ],
    [
        f64::from_bits(0x3fd1746887a8d64d),
        f64::from_bits(0x3fe0f9096bb98c7e),
        f64::from_bits(0x3fef81ecd4aa10e0),
    ],
    [
        f64::from_bits(0x3fd1590c0ad03d9b),
        f64::from_bits(0x3fe148e8a71de69b),
        f64::from_bits(0x3fefa858793dd97f),
    ],
    [
        f64::from_bits(0x3fd133b107746888),
        f64::from_bits(0x3fe19930be0ded29),
        f64::from_bits(0x3fefc6e6d9be4cd7),
    ],
    [
        f64::from_bits(0x3fd104d551d68c69),
        f64::from_bits(0x3fe1e9ccb7d41744),
        f64::from_bits(0x3fefddd6e04c0592),
    ],
    [
        f64::from_bits(0x3fd0cd20afa2f05a),
        f64::from_bits(0x3fe23abc947064ed),
        f64::from_bits(0x3fefed6777079e5a),
    ],
    [
        f64::from_bits(0x3fd08d3ae685db77),
        f64::from_bits(0x3fe28beb5b2d4d40),
        f64::from_bits(0x3feff5d78811b1d9),
    ],
    [
        f64::from_bits(0x3fd045a1cac08312),
        f64::from_bits(0x3fe2dd2f1a9fbe77),
        f64::from_bits(0x3feff77af640639d),
    ],
    [
        f64::from_bits(0x3fcfee4e26d4801f),
        f64::from_bits(0x3fe32e87d2c7b891),
        f64::from_bits(0x3feff27bb2fec56d),
    ],
    [
        f64::from_bits(0x3fcf443d46b26bf8),
        f64::from_bits(0x3fe37ff583a53b8e),
        f64::from_bits(0x3fefe72da122fad7),
    ],
    [
        f64::from_bits(0x3fce8f08461f9f02),
        f64::from_bits(0x3fe3d1633482be8c),
        f64::from_bits(0x3fefd5cfaacd9e84),
    ],
    [
        f64::from_bits(0x3fcdcf0307f23cc9),
        f64::from_bits(0x3fe422a6f3f52fc2),
        f64::from_bits(0x3fefbea0ba1f4b1f),
    ],
    [
        f64::from_bits(0x3fcd0678c0053e2d),
        f64::from_bits(0x3fe473c0c1fc8f32),
        f64::from_bits(0x3fefa1dfb9389b52),
    ],
    [
        f64::from_bits(0x3fcc35bd512ec6bd),
        f64::from_bits(0x3fe4c4b09e98dcdb),
        f64::from_bits(0x3fef7fe08aefb2ab),
    ],
    [
        f64::from_bits(0x3fcb5e74299d883c),
        f64::from_bits(0x3fe5156191148fda),
        f64::from_bits(0x3fef58cd20afa2f0),
    ],
    [
        f64::from_bits(0x3fca8198f1d3ed52),
        f64::from_bits(0x3fe565a9a8049668),
        f64::from_bits(0x3fef2ce4649906cd),
    ],
    [
        f64::from_bits(0x3fc9a07b352a8438),
        f64::from_bits(0x3fe5b59ddc1e7968),
        f64::from_bits(0x3feefc8f32378ab1),
    ],
    [
        f64::from_bits(0x3fc8bcbe61cffeb0),
        f64::from_bits(0x3fe605143bf72713),
        f64::from_bits(0x3feec7e28240b780),
    ],
    [
        f64::from_bits(0x3fc7d70a3d70a3d7),
        f64::from_bits(0x3fe653f7ced91687),
        f64::from_bits(0x3fee8f32378ab0c9),
    ],
    [
        f64::from_bits(0x3fc6f102363b2570),
        f64::from_bits(0x3fe6a25d8d79d0a6),
        f64::from_bits(0x3fee52d234eb9a17),
    ],
    [
        f64::from_bits(0x3fc60ba1f4b1ee24),
        f64::from_bits(0x3fe6f0068db8bac7),
        f64::from_bits(0x3fee12ec6bce8534),
    ],
    [
        f64::from_bits(0x3fc52839042d8c2a),
        f64::from_bits(0x3fe73d07c84b5dcc),
        f64::from_bits(0x3fedcfbfc6540cc8),
    ],
    [
        f64::from_bits(0x3fc447c30d306a2b),
        f64::from_bits(0x3fe7894c447c30d3),
        f64::from_bits(0x3fed89a027525461),
    ],
    [
        f64::from_bits(0x3fc36be37de939eb),
        f64::from_bits(0x3fe7d4bf0995aaf8),
        f64::from_bits(0x3fed40cc78e9f6a9),
    ],
    [
        f64::from_bits(0x3fc29595feda6613),
        f64::from_bits(0x3fe81f36262cba73),
        f64::from_bits(0x3fecf56eac860568),
    ],
    [
        f64::from_bits(0x3fc1c62a1b5c7cd9),
        f64::from_bits(0x3fe868c692f6e829),
        f64::from_bits(0x3feca7ef9db22d0e),
    ],
    [
        f64::from_bits(0x3fc0feef5ec80c74),
        f64::from_bits(0x3fe8b15b573eab36),
        f64::from_bits(0x3fec58793dd97f63),
    ],
    [
        f64::from_bits(0x3fc040e1719f7f8d),
        f64::from_bits(0x3fe8f8ca8198f1d4),
        f64::from_bits(0x3fec075f6fd21ff3),
    ],
    [
        f64::from_bits(0x3fbf1b4784230fd0),
        f64::from_bits(0x3fe93f290abb44e5),
        f64::from_bits(0x3febb4b72c5197a2),
    ],
    [
        f64::from_bits(0x3fbdcbbc2b94d940),
        f64::from_bits(0x3fe9844d013a92a3),
        f64::from_bits(0x3feb60fe47991bc5),
    ],
    [
        f64::from_bits(0x3fbc9667b5f1bef5),
        f64::from_bits(0x3fe9c8366516db0e),
        f64::from_bits(0x3feb0c49ba5e353f),
    ],
    [
        f64::from_bits(0x3fbb7d41743e963e),
        f64::from_bits(0x3fea0abb44e50c5f),
        f64::from_bits(0x3feab702602c9082),
    ],
    [
        f64::from_bits(0x3fba839042d8c2a4),
        f64::from_bits(0x3fea4bf0995aaf79),
        f64::from_bits(0x3fea613d31b9b670),
    ],
    [
        f64::from_bits(0x3fb9aaa3ad18d25f),
        f64::from_bits(0x3fea8bac710cb296),
        f64::from_bits(0x3fea0b630a91537a),
    ],
    [
        f64::from_bits(0x3fb8f5c28f5c28f6),
        f64::from_bits(0x3feac9d9d3458cd2),
        f64::from_bits(0x3fe9b59ddc1e7968),
    ],
    [
        f64::from_bits(0x3fb866e43aa79bbb),
        f64::from_bits(0x3feb068db8bac711),
        f64::from_bits(0x3fe9602c9081c2e3),
    ],
    [
        f64::from_bits(0x3fb8014f8b588e37),
        f64::from_bits(0x3feb4189374bc6a8),
        f64::from_bits(0x3fe90b630a91537a),
    ],
    [
        f64::from_bits(0x3fb7c6540cc78e9f),
        f64::from_bits(0x3feb7ae147ae147b),
        f64::from_bits(0x3fe8b780346dc5d6),
    ],
    [
        f64::from_bits(0x3fb7b9389b52007e),
        f64::from_bits(0x3febb280f12c27a6),
        f64::from_bits(0x3fe864c2f837b4a2),
    ],
    [
        f64::from_bits(0x3fb7dbf487fcb924),
        f64::from_bits(0x3febe8533b107747),
        f64::from_bits(0x3fe8136a400fba88),
    ],
    [
        f64::from_bits(0x3fb831ceaf251c19),
        f64::from_bits(0x3fec1c2e33eff195),
        f64::from_bits(0x3fe7c3c9eecbfb16),
    ],
    [
        f64::from_bits(0x3fb8bc169c23b795),
        f64::from_bits(0x3fec4e26d4801f75),
        f64::from_bits(0x3fe7760bf5d78812),
    ],
    [
        f64::from_bits(0x3fb97e132b55ef20),
        f64::from_bits(0x3fec7e28240b7803),
        f64::from_bits(0x3fe72a6f3f52fc26),
    ],
    [
        f64::from_bits(0x3fba79bbadc0980b),
        f64::from_bits(0x3fecac083126e979),
        f64::from_bits(0x3fe6e147ae147ae1),
    ],
    [
        f64::from_bits(0x3fbbafb7e90ff972),
        f64::from_bits(0x3fecd86ec17ebaf1),
        f64::from_bits(0x3fe697785729b281),
    ],
    [
        f64::from_bits(0x3fbd1e108c3f3e03),
        f64::from_bits(0x3fed03eea209aaa4),
        f64::from_bits(0x3fe649cf56eac860),
    ],
    [
        f64::from_bits(0x3fbec17ebaf10236),
        f64::from_bits(0x3fed2e87d2c7b891),
        f64::from_bits(0x3fe5f8a0902de00d),
    ],
    [
        f64::from_bits(0x3fc04c5974e65bea),
        f64::from_bits(0x3fed58255b035bd5),
        f64::from_bits(0x3fe5a400fba8826b),
    ],
    [
        f64::from_bits(0x3fc150331e3a7daa),
        f64::from_bits(0x3fed80c73abc9470),
        f64::from_bits(0x3fe54c447c30d307),
    ],
    [
        f64::from_bits(0x3fc26ba493c89f41),
        f64::from_bits(0x3feda858793dd97f),
        f64::from_bits(0x3fe4f1800a7c5ac4),
    ],
    [
        f64::from_bits(0x3fc39d0a67620ee9),
        f64::from_bits(0x3fedceee0f3cb3e5),
        f64::from_bits(0x3fe4941c8216c615),
    ],
    [
        f64::from_bits(0x3fc4e368f08461fa),
        f64::from_bits(0x3fedf47304039abf),
        f64::from_bits(0x3fe4342edbb59ddc),
    ],
    [
        f64::from_bits(0x3fc63e1869835159),
        f64::from_bits(0x3fee18d25edd0529),
        f64::from_bits(0x3fe3d1f601797cc4),
    ],
    [
        f64::from_bits(0x3fc7ab21815a07b3),
        f64::from_bits(0x3fee3c21187e7c07),
        f64::from_bits(0x3fe36d9be4cd7492),
    ],
    [
        f64::from_bits(0x3fc929dc725c3dee),
        f64::from_bits(0x3fee5e4a38327675),
        f64::from_bits(0x3fe307746887a8d6),
    ],
    [
        f64::from_bits(0x3fcab8f9b13165d4),
        f64::from_bits(0x3fee7f4dbdf8f473),
        f64::from_bits(0x3fe29fa97e132b56),
    ],
    [
        f64::from_bits(0x3fcc577d955714ba),
        f64::from_bits(0x3fee9f16b11c6d1e),
        f64::from_bits(0x3fe2366516db0dd8),
    ],
    [
        f64::from_bits(0x3fce03c4b09e98dd),
        f64::from_bits(0x3feebda5119ce076),
        f64::from_bits(0x3fe1cbe61cffeb07),
    ],
    [
        f64::from_bits(0x3fcfbd7b2031ceaf),
        f64::from_bits(0x3feedaf8df7a4e7b),
        f64::from_bits(0x3fe160807357e671),
    ],
    [
        f64::from_bits(0x3fd0c154c985f06f),
        f64::from_bits(0x3feef6fd21ff2e49),
        f64::from_bits(0x3fe0f43419e30015),
    ],
    [
        f64::from_bits(0x3fd1a97e132b55ef),
        f64::from_bits(0x3fef11c6d1e108c4),
        f64::from_bits(0x3fe08769ec2ce465),
    ],
    [
        f64::from_bits(0x3fd2963dc486ad2e),
        f64::from_bits(0x3fef2b40f66a5508),
        f64::from_bits(0x3fe01a4bdba0a527),
    ],
    [
        f64::from_bits(0x3fd3873ffac1d29e),
        f64::from_bits(0x3fef435696e58a33),
        f64::from_bits(0x3fdf5a07b352a844),
    ],
    [
        f64::from_bits(0x3fd47bdcf0307f24),
        f64::from_bits(0x3fef5a07b352a844),
        f64::from_bits(0x3fde7fa1a0cf1801),
    ],
    [
        f64::from_bits(0x3fd5736cdf266ba5),
        f64::from_bits(0x3fef6f544bb1af3a),
        f64::from_bits(0x3fdda5e353f7ced9),
    ],
    [
        f64::from_bits(0x3fd66d71f36262cc),
        f64::from_bits(0x3fef833c60029f17),
        f64::from_bits(0x3fdccd20afa2f05a),
    ],
    [
        f64::from_bits(0x3fd7696e58a32f45),
        f64::from_bits(0x3fef95aaf78feef6),
        f64::from_bits(0x3fdbf5d78811b1d9),
    ],
    [
        f64::from_bits(0x3fd866ba493c89f4),
        f64::from_bits(0x3fefa6a012599ed8),
        f64::from_bits(0x3fdb2085b18548aa),
    ],
    [
        f64::from_bits(0x3fd964d7f0ed3d86),
        f64::from_bits(0x3fefb61bb05faebc),
        f64::from_bits(0x3fda4d7f0ed3d85a),
    ],
    [
        f64::from_bits(0x3fda62f5989df117),
        f64::from_bits(0x3fefc408d8ec95c0),
        f64::from_bits(0x3fd97d1782d38477),
    ],
    [
        f64::from_bits(0x3fdb60bf5d78811b),
        f64::from_bits(0x3fefd0678c0053e3),
        f64::from_bits(0x3fd8b020c49ba5e3),
    ],
    [
        f64::from_bits(0x3fdc5d8d79d0a676),
        f64::from_bits(0x3fefdb37c99ae925),
        f64::from_bits(0x3fd7e69ad42c3c9f),
    ],
    [
        f64::from_bits(0x3fdd58b827fa1a0d),
        f64::from_bits(0x3fefe4649906cca3),
        f64::from_bits(0x3fd7212d77318fc5),
    ],
    [
        f64::from_bits(0x3fde51eb851eb852),
        f64::from_bits(0x3fefebedfa43fe5d),
        f64::from_bits(0x3fd6605681ecd4aa),
    ],
    [
        f64::from_bits(0x3fdf4855da272863),
        f64::from_bits(0x3feff1e8e6080735),
        f64::from_bits(0x3fd5a469d7342edc),
    ],
    [
        f64::from_bits(0x3fe01da7b0b39192),
        f64::from_bits(0x3feff61672324c83),
        f64::from_bits(0x3fd4edbb59ddc1e8),
    ],
    [
        f64::from_bits(0x3fe095421c044285),
        f64::from_bits(0x3feff8a0902de00d),
        f64::from_bits(0x3fd43cf2cf95d4e9),
    ],
    [
        f64::from_bits(0x3fe10aa64c2f837b),
        f64::from_bits(0x3feff95d4e8fb00c),
        f64::from_bits(0x3fd392641b328b6e),
    ],
    [
        f64::from_bits(0x3fe17d955714b9cb),
        f64::from_bits(0x3feff861a60d4563),
        f64::from_bits(0x3fd2ee8d10f51aca),
    ],
    [
        f64::from_bits(0x3fe1eda661283904),
        f64::from_bits(0x3feff5989df1172f),
        f64::from_bits(0x3fd25197a24894c4),
    ],
    [
        f64::from_bits(0x3fe25aaf78feef5f),
        f64::from_bits(0x3feff102363b2570),
        f64::from_bits(0x3fd1bc558644523f),
    ],
    [
        f64::from_bits(0x3fe2c447c30d306a),
        f64::from_bits(0x3fefea9e6eeb7026),
        f64::from_bits(0x3fd12ef0ae536502),
    ],
    [
        f64::from_bits(0x3fe32a454de7ea60),
        f64::from_bits(0x3fefe2584f4c6e6e),
        f64::from_bits(0x3fd0aa10e0221427),
    ],
    [
        f64::from_bits(0x3fe38c5436b8f9b1),
        f64::from_bits(0x3fefd82fd75e2047),
        f64::from_bits(0x3fd02de00d1b7176),
    ],
    [
        f64::from_bits(0x3fe3ea209aaa3ad2),
        f64::from_bits(0x3fefcc100e6afcce),
        f64::from_bits(0x3fcf760bf5d78812),
    ],
    [
        f64::from_bits(0x3fe4436b8f9b1316),
        f64::from_bits(0x3fefbe0ded288ce7),
        f64::from_bits(0x3fcea3ad18d25edd),
    ],
    [
        f64::from_bits(0x3fe49888f861a60d),
        f64::from_bits(0x3fefadff822bbecb),
        f64::from_bits(0x3fcde54b48d3ae68),
    ],
    [
        f64::from_bits(0x3fe4ed1394317acc),
        f64::from_bits(0x3fef9ba5e353f7cf),
        f64::from_bits(0x3fcd3a92a3055326),
    ],
    [
        f64::from_bits(0x3fe541c8216c6152),
        f64::from_bits(0x3fef87160956c0d7),
        f64::from_bits(0x3fcca2339c0ebee0),
    ],
    [
        f64::from_bits(0x3fe5967caea747d8),
        f64::from_bits(0x3fef704ff43419e3),
        f64::from_bits(0x3fcc1bda5119ce07),
    ],
    [
        f64::from_bits(0x3fe5eb074a771c97),
        f64::from_bits(0x3fef5753a3ec02f3),
        f64::from_bits(0x3fcba68b19a415f4),
    ],
    [
        f64::from_bits(0x3fe63f7ced916873),
        f64::from_bits(0x3fef3c36113404ea),
        f64::from_bits(0x3fcb419e30014f8b),
    ],
    [
        f64::from_bits(0x3fe693b3a68b19a4),
        f64::from_bits(0x3fef1f212d773190),
        f64::from_bits(0x3fcaebc408d8ec96),
    ],
    [
        f64::from_bits(0x3fe6e7967caea748),
        f64::from_bits(0x3fef000000000000),
        f64::from_bits(0x3fcaa4a8c154c986),
    ],
    [
        f64::from_bits(0x3fe73b107746887b),
        f64::from_bits(0x3feeded288ce703b),
        f64::from_bits(0x3fca6b50b0f27bb3),
    ],
    [
        f64::from_bits(0x3fe78e219652bd3c),
        f64::from_bits(0x3feebbd7b2031ceb),
        f64::from_bits(0x3fca3ec02f2f9874),
    ],
    [
        f64::from_bits(0x3fe7e09fe86833c6),
        f64::from_bits(0x3fee96fa82e87d2c),
        f64::from_bits(0x3fca1ea35935fc3b),
    ],
    [
        f64::from_bits(0x3fe8328b6d86ec18),
        f64::from_bits(0x3fee704ff43419e3),
        f64::from_bits(0x3fca09fe86833c60),
    ],
    [
        f64::from_bits(0x3fe883ba3443d46b),
        f64::from_bits(0x3fee47d805e5f30e),
        f64::from_bits(0x3fc9ff822bbecaac),
    ],
    [
        f64::from_bits(0x3fe8d441355475a3),
        f64::from_bits(0x3fee1dbca9691a76),
        f64::from_bits(0x3fc9ff2e48e8a71e),
    ],
    [
        f64::from_bits(0x3fe923e186983516),
        f64::from_bits(0x3fedf1fddebd9019),
        f64::from_bits(0x3fca07b352a84381),
    ],
    [
        f64::from_bits(0x3fe9729b280f12c2),
        f64::from_bits(0x3fedc49ba5e353f8),
        f64::from_bits(0x3fca1815a07b352b),
    ],
    [
        f64::from_bits(0x3fe9c059210385c6),
        f64::from_bits(0x3fed95aaf78feef6),
        f64::from_bits(0x3fca30014f8b588e),
    ],
    [
        f64::from_bits(0x3fea0d1b71758e22),
        f64::from_bits(0x3fed6540cc78e9f7),
        f64::from_bits(0x3fca4e7ab7564303),
    ],
    [
        f64::from_bits(0x3fea58b827fa1a0d),
        f64::from_bits(0x3fed33721d53cddd),
        f64::from_bits(0x3fca72da122fad6d),
    ],
    [
        f64::from_bits(0x3feaa31a4bdba0a5),
        f64::from_bits(0x3fed0029f16b11c7),
        f64::from_bits(0x3fca9bcfd4bf0996),
    ],
    [
        f64::from_bits(0x3feaec2ce4649907),
        f64::from_bits(0x3feccba732df505d),
        f64::from_bits(0x3fcac9081c2e33f0),
    ],
    [
        f64::from_bits(0x3feb3404ea4a8c15),
        f64::from_bits(0x3fec95bff04577d9),
        f64::from_bits(0x3fcaf9873ffac1d3),
    ],
    [
        f64::from_bits(0x3feb7a4e7ab75643),
        f64::from_bits(0x3fec5e9e1b089a02),
        f64::from_bits(0x3fcb2ca57a786c22),
    ],
    [
        f64::from_bits(0x3febbf3387160957),
        f64::from_bits(0x3fec2656abde3fbc),
        f64::from_bits(0x3fcb61672324c836),
    ],
    [
        f64::from_bits(0x3fec027525460aa6),
        f64::from_bits(0x3febece9a2c66905),
        f64::from_bits(0x3fcb972474538ef3),
    ],
    [
        f64::from_bits(0x3fec441355475a32),
        f64::from_bits(0x3febb26bf8769ec3),
        f64::from_bits(0x3fcbcce1c58255b0),
    ],
    [
        f64::from_bits(0x3fec840e1719f7f9),
        f64::from_bits(0x3feb76ddaceee0f4),
        f64::from_bits(0x3fcc01a36e2eb1c4),
    ],
    [
        f64::from_bits(0x3fecc226809d4952),
        f64::from_bits(0x3feb3a53b8e4b87c),
        f64::from_bits(0x3fcc35696e58a32f),
    ],
    [
        f64::from_bits(0x3fecfe5c91d14e3c),
        f64::from_bits(0x3feafce3150dae3e),
        f64::from_bits(0x3fcc669057d1782d),
    ],
    [
        f64::from_bits(0x3fed38b04ab606b8),
        f64::from_bits(0x3feabe8bc169c23b),
        f64::from_bits(0x3fcc947064ece9a3),
    ],
    [
        f64::from_bits(0x3fed70f7b9e060fe),
        f64::from_bits(0x3fea7f77af64063a),
        f64::from_bits(0x3fccbe61cffeb075),
    ],
    [
        f64::from_bits(0x3feda732df505d10),
        f64::from_bits(0x3fea3f7ced916873),
        f64::from_bits(0x3fcce3bcd35a8588),
    ],
    [
        f64::from_bits(0x3feddb37c99ae925),
        f64::from_bits(0x3fe9feda66128390),
        f64::from_bits(0x3fcd0385c67dfe33),
    ],
    [
        f64::from_bits(0x3fee0d0678c0053e),
        f64::from_bits(0x3fe9bd9018e75793),
        f64::from_bits(0x3fcd1cc100e6afcd),
    ],
    [
        f64::from_bits(0x3fee3c89f40a2878),
        f64::from_bits(0x3fe97b9e060fe47a),
        f64::from_bits(0x3fcd2f1a9fbe76c9),
    ],
    [
        f64::from_bits(0x3fee69984a0e410b),
        f64::from_bits(0x3fe9392e1ef73c0c),
        f64::from_bits(0x3fcd394317acc4f0),
    ],
    [
        f64::from_bits(0x3fee94467381d7dc),
        f64::from_bits(0x3fe8f62b6ae7d567),
        f64::from_bits(0x3fcd3ae685db76b4),
    ],
    [
        f64::from_bits(0x3feebc558644523f),
        f64::from_bits(0x3fe8b2aae297396d),
        f64::from_bits(0x3fcd32b55ef1fddf),
    ],
    [
        f64::from_bits(0x3feee1da7b0b3919),
        f64::from_bits(0x3fe86ec17ebaf102),
        f64::from_bits(0x3fcd2007dd441355),
    ],
    [
        f64::from_bits(0x3fef049667b5f1bf),
        f64::from_bits(0x3fe82a843808850a),
        f64::from_bits(0x3fcd02363b256ffc),
    ],
    [
        f64::from_bits(0x3fef24b33daf8df8),
        f64::from_bits(0x3fe7e5f30e7ff584),
        f64::from_bits(0x3fccd898b2e9ccb8),
    ],
    [
        f64::from_bits(0x3fef41dd1a21ea36),
        f64::from_bits(0x3fe7a122fad6cb53),
        f64::from_bits(0x3fcca1dfb9389b52),
    ],
    [
        f64::from_bits(0x3fef5c28f5c28f5c),
        f64::from_bits(0x3fe75c28f5c28f5c),
        f64::from_bits(0x3fcc5db76b3bb83d),
    ],
    [
        f64::from_bits(0x3fef73d5bab21816),
        f64::from_bits(0x3fe715b573eab368),
        f64::from_bits(0x3fcc0e1719f7f8cb),
    ],
    [
        f64::from_bits(0x3fef892253111f0c),
        f64::from_bits(0x3fe6cccccccccccd),
        f64::from_bits(0x3fcbb645a1cac083),
    ],
    [
        f64::from_bits(0x3fef9c23b7952d23),
        f64::from_bits(0x3fe6816f0068db8c),
        f64::from_bits(0x3fcb5696e58a32f4),
    ],
    [
        f64::from_bits(0x3fefaceee0f3cb3e),
        f64::from_bits(0x3fe633c60029f16b),
        f64::from_bits(0x3fcaef5ec80c73ac),
    ],
    [
        f64::from_bits(0x3fefbb6ed677707a),
        f64::from_bits(0x3fe5e3fbbd7b2032),
        f64::from_bits(0x3fca80f12c27a637),
    ],
    [
        f64::from_bits(0x3fefc7cd898b2e9d),
        f64::from_bits(0x3fe59210385c67e0),
        f64::from_bits(0x3fca0ba1f4b1ee24),
    ],
    [
        f64::from_bits(0x3fefd1f601797cc4),
        f64::from_bits(0x3fe53e5753a3ec03),
        f64::from_bits(0x3fc99018e757928e),
    ],
    [
        f64::from_bits(0x3fefd9e83e425aee),
        f64::from_bits(0x3fe4e8bc169c23b8),
        f64::from_bits(0x3fc90efdc9c4da90),
    ],
    [
        f64::from_bits(0x3fefdfce3150dae4),
        f64::from_bits(0x3fe49192641b328b),
        f64::from_bits(0x3fc887fcb923a29c),
    ],
    [
        f64::from_bits(0x3fefe3a7daa4fca4),
        f64::from_bits(0x3fe438c5436b8f9b),
        f64::from_bits(0x3fc7fc115df6555c),
    ],
    [
        f64::from_bits(0x3fefe5604189374c),
        f64::from_bits(0x3fe3dea897635e74),
        f64::from_bits(0x3fc76b8f9b13165d),
    ],
    [
        f64::from_bits(0x3fefe52157689ca2),
        f64::from_bits(0x3fe3833c60029f17),
        f64::from_bits(0x3fc6d6777079e59f),
    ],
    [
        f64::from_bits(0x3fefe2d6238da3c2),
        f64::from_bits(0x3fe3269595feda66),
        f64::from_bits(0x3fc63dc486ad2dcb),
    ],
    [
        f64::from_bits(0x3fefdea897635e74),
        f64::from_bits(0x3fe2c8f32378ab0d),
        f64::from_bits(0x3fc5a176ddaceee1),
    ],
    [
        f64::from_bits(0x3fefd86ec17ebaf1),
        f64::from_bits(0x3fe26a6a012599ed),
        f64::from_bits(0x3fc501e2584f4c6e),
    ],
    [
        f64::from_bits(0x3fefd0678c0053e3),
        f64::from_bits(0x3fe20b242070b8d0),
        f64::from_bits(0x3fc460029f16b11c),
    ],
    [
        f64::from_bits(0x3fefc669057d1783),
        f64::from_bits(0x3fe1ab21815a07b3),
        f64::from_bits(0x3fc3bbd7b2031ceb),
    ],
    [
        f64::from_bits(0x3fefba9d1f601798),
        f64::from_bits(0x3fe14aa10e022142),
        f64::from_bits(0x3fc315b573eab368),
    ],
    [
        f64::from_bits(0x3fefad03d9a95422),
        f64::from_bits(0x3fe0e9ccb7d41744),
        f64::from_bits(0x3fc26defc7a39820),
    ],
    [
        f64::from_bits(0x3fef9d9d3458cd21),
        f64::from_bits(0x3fe0888f861a60d4),
        f64::from_bits(0x3fc1c52e72da1230),
    ],
    [
        f64::from_bits(0x3fef8c7e28240b78),
        f64::from_bits(0x3fe0273d5bab2181),
        f64::from_bits(0x3fc11c193b3a68b2),
    ],
    [
        f64::from_bits(0x3fef79a6b50b0f28),
        f64::from_bits(0x3fdf8bd66277c45d),
        f64::from_bits(0x3fc0725c3dee7818),
    ],
    [
        f64::from_bits(0x3fef6501e2584f4c),
        f64::from_bits(0x3fdec95bff04577e),
        f64::from_bits(0x3fbf91e646f15619),
    ],
    [
        f64::from_bits(0x3fef4ece9a2c6690),
        f64::from_bits(0x3fde075f6fd21ff3),
        f64::from_bits(0x3fbe40639d5e4a38),
    ],
    [
        f64::from_bits(0x3fef36e2eb1c432d),
        f64::from_bits(0x3fdd460aa64c2f83),
        f64::from_bits(0x3fbcf0d844d013a9),
    ],
    [
        f64::from_bits(0x3fef1d68c692f6e8),
        f64::from_bits(0x3fdc858793dd97f6),
        f64::from_bits(0x3fbba3ec02f2f987),
    ],
    [
        f64::from_bits(0x3fef02602c9081c3),
        f64::from_bits(0x3fdbc67dfe32a066),
        f64::from_bits(0x3fba5a469d7342ee),
    ],
    [
        f64::from_bits(0x3feee5c91d14e3bd),
        f64::from_bits(0x3fdb08c3f3e0370d),
        f64::from_bits(0x3fb915379fa97e13),
    ],
    [
        f64::from_bits(0x3feec7a398201cd6),
        f64::from_bits(0x3fda4cad57bc7f78),
        f64::from_bits(0x3fb7d566cf41f213),
    ],
    [
        f64::from_bits(0x3feea8049667b5f2),
        f64::from_bits(0x3fd992b7fe08aefb),
        f64::from_bits(0x3fb69b7bf1e8e608),
    ],
    [
        f64::from_bits(0x3fee870110a137f4),
        f64::from_bits(0x3fd8dae3e6c4c597),
        f64::from_bits(0x3fb5681ecd4aa10e),
    ],
    [
        f64::from_bits(0x3fee64840e1719f8),
        f64::from_bits(0x3fd82584f4c6e6da),
        f64::from_bits(0x3fb43c9eecbfb15b),
    ],
    [
        f64::from_bits(0x3fee408d8ec95bff),
        f64::from_bits(0x3fd772c5197a2489),
        f64::from_bits(0x3fb319a415f45e0b),
    ],
    [
        f64::from_bits(0x3fee1b5c7cd898b3),
        f64::from_bits(0x3fd6c322291fb3fa),
        f64::from_bits(0x3fb1ffd60e94ee39),
    ],
    [
        f64::from_bits(0x3fedf4b1ee243569),
        f64::from_bits(0x3fd6169c23b7952d),
        f64::from_bits(0x3fb0efdc9c4da900),
    ],
    [
        f64::from_bits(0x3fedcccccccccccd),
        f64::from_bits(0x3fd56db0dd82fd76),
        f64::from_bits(0x3fafd60e94ee392e),
    ],
    [
        f64::from_bits(0x3feda3ad18d25edd),
        f64::from_bits(0x3fd4c8605681ecd5),
        f64::from_bits(0x3fade2ac322291fb),
    ],
    [
        f64::from_bits(0x3fed793dd97f62b7),
        f64::from_bits(0x3fd4272862f5989e),
        f64::from_bits(0x3fac083126e978d5),
    ],
    [
        f64::from_bits(0x3fed4d940789613d),
        f64::from_bits(0x3fd38a0902de00d2),
        f64::from_bits(0x3faa493c89f40a28),
    ],
    [
        f64::from_bits(0x3fed20afa2f05a71),
        f64::from_bits(0x3fd2f1800a7c5ac4),
        f64::from_bits(0x3fa8a5ce5b4245f6),
    ],
    [
        f64::from_bits(0x3fecf2ba9d1f6018),
        f64::from_bits(0x3fd25d8d79d0a676),
        f64::from_bits(0x3fa71f36262cba73),
    ],
    [
        f64::from_bits(0x3fecc39ffd60e94f),
        f64::from_bits(0x3fd1ceaf251c193b),
        f64::from_bits(0x3fa5b9628cbd1245),
    ],
    [
        f64::from_bits(0x3fec934acaff6d33),
        f64::from_bits(0x3fd1449129888f86),
        f64::from_bits(0x3fa471b4784230fd),
    ],
    [
        f64::from_bits(0x3fec6191148fd9fd),
        f64::from_bits(0x3fd0bcbe61cffeb0),
        f64::from_bits(0x3fa33721d53cddd7),
    ],
    [
        f64::from_bits(0x3fec2e5de15ca6ca),
        f64::from_bits(0x3fd036b8f9b13166),
        f64::from_bits(0x3fa2070b8cfbfc65),
    ],
    [
        f64::from_bits(0x3febf99c38b04ab6),
        f64::from_bits(0x3fcf64adff822bbf),
        f64::from_bits(0x3fa0e1719f7f8ca8),
    ],
    [
        f64::from_bits(0x3febc36113404ea5),
        f64::from_bits(0x3fce5fd8adab9f56),
        f64::from_bits(0x3f9f8f47304039ac),
    ],
    [
        f64::from_bits(0x3feb8b97785729b3),
        f64::from_bits(0x3fcd5e9e1b089a02),
        f64::from_bits(0x3f9d70a3d70a3d71),
    ],
    [
        f64::from_bits(0x3feb525460aa64c3),
        f64::from_bits(0x3fcc60aa64c2f838),
        f64::from_bits(0x3f9b69984a0e410b),
    ],
    [
        f64::from_bits(0x3feb1782d38476f3),
        f64::from_bits(0x3fcb66a550870111),
        f64::from_bits(0x3f997785729b280f),
    ],
    [
        f64::from_bits(0x3feadb37c99ae925),
        f64::from_bits(0x3fca6fe718a86d72),
        f64::from_bits(0x3f979a6b50b0f27c),
    ],
    [
        f64::from_bits(0x3fea9d7342edbb5a),
        f64::from_bits(0x3fc97cc39ffd60e9),
        f64::from_bits(0x3f95d249e44fa051),
    ],
    [
        f64::from_bits(0x3fea5e2046c764ae),
        f64::from_bits(0x3fc88d8ec95bff04),
        f64::from_bits(0x3f9421c044284dfd),
    ],
    [
        f64::from_bits(0x3fea1d53cddd6e05),
        f64::from_bits(0x3fc7a1a0cf1800a8),
        f64::from_bits(0x3f92862f5989df11),
    ],
    [
        f64::from_bits(0x3fe9db0dd82fd75e),
        f64::from_bits(0x3fc6b94d94078961),
        f64::from_bits(0x3f90ff972474538f),
    ],
    [
        f64::from_bits(0x3fe997396d0917d7),
        f64::from_bits(0x3fc5d495182a9931),
        f64::from_bits(0x3f8f212d77318fc5),
    ],
    [
        f64::from_bits(0x3fe951eb851eb852),
        f64::from_bits(0x3fc4f3775b813016),
        f64::from_bits(0x3f8c67dfe32a0664),
    ],
    [
        f64::from_bits(0x3fe90b0f27bb2fec),
        f64::from_bits(0x3fc4164840e1719f),
        f64::from_bits(0x3f89e30014f8b589),
    ],
    [
        f64::from_bits(0x3fe8c2b94d940789),
        f64::from_bits(0x3fc33c60029f16b1),
        f64::from_bits(0x3f8782d38476f2a6),
    ],
    [
        f64::from_bits(0x3fe878e9f6a93f29),
        f64::from_bits(0x3fc26612839042d9),
        f64::from_bits(0x3f8551d68c692f6f),
    ],
    [
        f64::from_bits(0x3fe82d8c2a454de8),
        f64::from_bits(0x3fc1935fc3b4f616),
        f64::from_bits(0x3f834acaff6d3309),
    ],
    [
        f64::from_bits(0x3fe7e09fe86833c6),
        f64::from_bits(0x3fc0c3f3e0370cdd),
        f64::from_bits(0x3f816db0dd82fd76),
    ],
    [
        f64::from_bits(0x3fe7924f227d028a),
        f64::from_bits(0x3fbff0ed3d859c8d),
        f64::from_bits(0x3f7f7f8ca8198f1d),
    ],
    [
        f64::from_bits(0x3fe7426fe718a86d),
        f64::from_bits(0x3fbe612839042d8c),
        f64::from_bits(0x3f7c779a6b50b0f2),
    ],
    [
        f64::from_bits(0x3fe6f102363b2570),
        f64::from_bits(0x3fbcd898b2e9ccb8),
        f64::from_bits(0x3f79c38b04ab606b),
    ],
    [
        f64::from_bits(0x3fe69e1b089a0275),
        f64::from_bits(0x3fbb573eab367a10),
        f64::from_bits(0x3f77635e74299d88),
    ],
    [
        f64::from_bits(0x3fe649ba5e353f7d),
        f64::from_bits(0x3fb9dc725c3dee78),
        f64::from_bits(0x3f756191148fd9fd),
    ],
    [
        f64::from_bits(0x3fe5f3e0370cdc87),
        f64::from_bits(0x3fb869835158b828),
        f64::from_bits(0x3f73b3a68b19a416),
    ],
    [
        f64::from_bits(0x3fe59c779a6b50b1),
        f64::from_bits(0x3fb6fd21ff2e48e9),
        f64::from_bits(0x3f72641b328b6d87),
    ],
    [
        f64::from_bits(0x3fe5438088509bfa),
        f64::from_bits(0x3fb5989df1172ef1),
        f64::from_bits(0x3f715df6555c52e7),
    ],
    [
        f64::from_bits(0x3fe4e90ff9724745),
        f64::from_bits(0x3fb43aa79bbadc0a),
        f64::from_bits(0x3f70b630a91537a0),
    ],
    [
        f64::from_bits(0x3fe48d25edd05293),
        f64::from_bits(0x3fb2e48e8a71de6a),
        f64::from_bits(0x3f706cca2db61bb0),
    ],
    [
        f64::from_bits(0x3fe42fad6cb53501),
        f64::from_bits(0x3fb1950331e3a7db),
        f64::from_bits(0x3f706cca2db61bb0),
    ],
    [
        f64::from_bits(0x3fe3d0bb6ed67770),
        f64::from_bits(0x3fb04cad57bc7f78),
        f64::from_bits(0x3f70cb295e9e1b09),
    ],
    [
        f64::from_bits(0x3fe3704ff43419e3),
        f64::from_bits(0x3fae1869835158b8),
        f64::from_bits(0x3f717d6b65a9a805),
    ],
    [
        f64::from_bits(0x3fe30e5604189375),
        f64::from_bits(0x3faba493c89f40a3),
        f64::from_bits(0x3f728e0c9d9d3459),
    ],
    [
        f64::from_bits(0x3fe2aae297396d09),
        f64::from_bits(0x3fa93f290abb44e5),
        f64::from_bits(0x3f73e81450efdc9c),
    ],
    [
        f64::from_bits(0x3fe245e0b4e11dbd),
        f64::from_bits(0x3fa6e82949a56580),
        f64::from_bits(0x3f75aaf78feef5ed),
    ],
    [
        f64::from_bits(0x3fe1df6555c52e73),
        f64::from_bits(0x3fa49f94855da273),
        f64::from_bits(0x3f77b7414a4d2b2c),
    ],
    [
        f64::from_bits(0x3fe1777079e59f2c),
        f64::from_bits(0x3fa2656abde3fbbd),
        f64::from_bits(0x3f7a21ea35935fc4),
    ],
    [
        f64::from_bits(0x3fe10ded288ce704),
        f64::from_bits(0x3fa039abf3387161),
        f64::from_bits(0x3f7ce075f6fd21ff),
    ],
    [
        f64::from_bits(0x3fe0a2f05a708ede),
        f64::from_bits(0x3f9c38b04ab606b8),
        f64::from_bits(0x3f7ff2e48e8a71de),
    ],
    [
        f64::from_bits(0x3fe0366516db0dd8),
        f64::from_bits(0x3f981adea897635e),
        f64::from_bits(0x3f81ac9afe1da7b1),
    ],
    [
        f64::from_bits(0x3fdf90c0ad03d9a9),
        f64::from_bits(0x3f9419e30014f8b6),
        f64::from_bits(0x3f838ef34d6a161e),
    ],
    [
        f64::from_bits(0x3fdeb1c432ca57a8),
        f64::from_bits(0x3f9035bd512ec6bd),
        f64::from_bits(0x3f859b3d07c84b5e),
    ],
];

/// Returns MATLAB R2022b-compatible `parula(m)` RGB rows.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn parula_r2022b(length: usize) -> Vec<[f64; 3]> {
    resample_table(&PARULA_R2022B, length)
}

/// The complete predefined colormap catalog available in MATLAB R2022b.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PredefinedColormap {
    /// MATLAB's default blue-green-yellow scientific map.
    Parula,
    /// Perceptually smoother rainbow map introduced before R2022b.
    Turbo,
    /// Hue-saturation-value color wheel.
    Hsv,
    /// Black-red-yellow-white heat map.
    Hot,
    /// Cyan-to-magenta map.
    Cool,
    /// Magenta-to-yellow map.
    Spring,
    /// Greenish seasonal map.
    Summer,
    /// Red-to-yellow map.
    Autumn,
    /// Blue-to-green map.
    Winter,
    /// Grayscale map.
    Gray,
    /// Grayscale map with a blue tint.
    Bone,
    /// Copper-toned grayscale map.
    Copper,
    /// Pastel pink heat map.
    Pink,
    /// Classic blue-cyan-yellow-red map.
    Jet,
    /// Axes color-order map.
    Lines,
    /// RGB cube map with grayscale tail.
    Colorcube,
    /// Repeating six-color prism map.
    Prism,
    /// Repeating red-white-blue-black map.
    Flag,
    /// All-white map.
    White,
    /// Fixed 16-color VGA palette.
    Vga,
}

impl PredefinedColormap {
    /// All predefined colormaps, in the R2022b documentation order.
    pub const ALL: [Self; 20] = [
        Self::Parula,
        Self::Turbo,
        Self::Hsv,
        Self::Hot,
        Self::Cool,
        Self::Spring,
        Self::Summer,
        Self::Autumn,
        Self::Winter,
        Self::Gray,
        Self::Bone,
        Self::Copper,
        Self::Pink,
        Self::Jet,
        Self::Lines,
        Self::Colorcube,
        Self::Prism,
        Self::Flag,
        Self::White,
        Self::Vga,
    ];

    /// Resolves a case-insensitive predefined name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.name().eq_ignore_ascii_case(name))
    }

    /// Returns the canonical lower-case MATLAB function name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Parula => "parula",
            Self::Turbo => "turbo",
            Self::Hsv => "hsv",
            Self::Hot => "hot",
            Self::Cool => "cool",
            Self::Spring => "spring",
            Self::Summer => "summer",
            Self::Autumn => "autumn",
            Self::Winter => "winter",
            Self::Gray => "gray",
            Self::Bone => "bone",
            Self::Copper => "copper",
            Self::Pink => "pink",
            Self::Jet => "jet",
            Self::Lines => "lines",
            Self::Colorcube => "colorcube",
            Self::Prism => "prism",
            Self::Flag => "flag",
            Self::White => "white",
            Self::Vga => "vga",
        }
    }

    /// Returns whether the function accepts an explicit output length.
    #[must_use]
    pub const fn supports_length(self) -> bool {
        !matches!(self, Self::Vga)
    }

    /// Generates this map at the requested length.
    ///
    /// `Vga` is fixed at 16 rows and ignores `length`; callers should use
    /// [`Self::supports_length`] to enforce its public no-input contract.
    #[must_use]
    pub fn generate(self, length: usize) -> Vec<[f64; 3]> {
        predefined_colormap_r2022b(self, length)
    }
}

/// Generates one predefined MATLAB R2022b colormap.
#[must_use]
pub fn predefined_colormap_r2022b(colormap: PredefinedColormap, length: usize) -> Vec<[f64; 3]> {
    match colormap {
        PredefinedColormap::Parula => resample_table(&PARULA_R2022B, length),
        PredefinedColormap::Turbo => resample_table(&TURBO_R2022B, length),
        PredefinedColormap::Hsv => hsv(length),
        PredefinedColormap::Hot => hot(length),
        PredefinedColormap::Cool => linear_map(length, |value| [value, 1.0 - value, 1.0]),
        PredefinedColormap::Spring => linear_map(length, |value| [1.0, value, 1.0 - value]),
        PredefinedColormap::Summer => linear_map(length, |value| [value, 0.5 + value / 2.0, 0.4]),
        PredefinedColormap::Autumn => linear_map(length, |value| [1.0, value, 0.0]),
        PredefinedColormap::Winter => linear_map(length, |value| [0.0, value, 1.0 - value / 2.0]),
        PredefinedColormap::Gray => gray(length),
        PredefinedColormap::Bone => bone(length),
        PredefinedColormap::Copper => copper(length),
        PredefinedColormap::Pink => pink(length),
        PredefinedColormap::Jet => jet(length),
        PredefinedColormap::Lines => repeat_palette(length, &LINES_PALETTE),
        PredefinedColormap::Colorcube => colorcube(length),
        PredefinedColormap::Prism => repeat_palette(length, &PRISM_PALETTE),
        PredefinedColormap::Flag => repeat_palette(length, &FLAG_PALETTE),
        PredefinedColormap::White => vec![[1.0; 3]; length],
        PredefinedColormap::Vga => VGA_PALETTE.to_vec(),
    }
}

const LINES_PALETTE: [[f64; 3]; 7] = [
    [0.0, 0.447, 0.741],
    [0.85, 0.325, 0.098],
    [0.929, 0.694, 0.125],
    [0.494, 0.184, 0.556],
    [0.466, 0.674, 0.188],
    [0.301, 0.745, 0.933],
    [0.635, 0.078, 0.184],
];

const PRISM_PALETTE: [[f64; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [1.0, 0.5, 0.0],
    [1.0, 1.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [2.0 / 3.0, 0.0, 1.0],
];

const FLAG_PALETTE: [[f64; 3]; 4] = [
    [1.0, 0.0, 0.0],
    [1.0, 1.0, 1.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, 0.0],
];

const VGA_PALETTE: [[f64; 3]; 16] = [
    [1.0, 1.0, 1.0],
    [0.75, 0.75, 0.75],
    [1.0, 0.0, 0.0],
    [1.0, 1.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 1.0, 1.0],
    [0.0, 0.0, 1.0],
    [1.0, 0.0, 1.0],
    [0.0, 0.0, 0.0],
    [0.5, 0.5, 0.5],
    [0.5, 0.0, 0.0],
    [0.5, 0.5, 0.0],
    [0.0, 0.5, 0.0],
    [0.0, 0.5, 0.5],
    [0.0, 0.0, 0.5],
    [0.5, 0.0, 0.5],
];

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn resample_table(table: &[[f64; 3]; DEFAULT_COLORMAP_LENGTH], length: usize) -> Vec<[f64; 3]> {
    match length {
        0 => Vec::new(),
        1 => vec![table[DEFAULT_COLORMAP_LENGTH - 1]],
        DEFAULT_COLORMAP_LENGTH => table.to_vec(),
        _ => {
            let denominator = (length - 1) as f64;
            (0..length)
                .map(|index| {
                    let position =
                        index as f64 * (DEFAULT_COLORMAP_LENGTH - 1) as f64 / denominator;
                    let lower = position.floor() as usize;
                    let upper = (lower + 1).min(DEFAULT_COLORMAP_LENGTH - 1);
                    let fraction = position - lower as f64;
                    let lower_weight = 1.0 - fraction;
                    [
                        table[lower][0] * lower_weight + table[upper][0] * fraction,
                        table[lower][1] * lower_weight + table[upper][1] * fraction,
                        table[lower][2] * lower_weight + table[upper][2] * fraction,
                    ]
                })
                .collect()
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn linear_map(length: usize, map: impl Fn(f64) -> [f64; 3]) -> Vec<[f64; 3]> {
    match length {
        0 => Vec::new(),
        1 => vec![map(0.0)],
        _ => {
            let denominator = (length - 1) as f64;
            (0..length)
                .map(|index| map(index as f64 / denominator))
                .collect()
        }
    }
}

fn gray(length: usize) -> Vec<[f64; 3]> {
    linear_map(length, |value| [value; 3])
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn hsv(length: usize) -> Vec<[f64; 3]> {
    (0..length)
        .map(|index| {
            let scaled = index as f64 * 6.0 / length as f64;
            let sector = scaled.floor() as usize;
            let fraction = scaled - sector as f64;
            match sector {
                0 => [1.0, fraction, 0.0],
                1 => [1.0 - fraction, 1.0, 0.0],
                2 => [0.0, 1.0, fraction],
                3 => [0.0, 1.0 - fraction, 1.0],
                4 => [fraction, 0.0, 1.0],
                _ => [1.0, 0.0, 1.0 - fraction],
            }
        })
        .collect()
}

#[allow(clippy::cast_precision_loss)]
fn hot(length: usize) -> Vec<[f64; 3]> {
    match length {
        0 => Vec::new(),
        1 => vec![[1.0; 3]],
        2 => vec![[1.0, 1.0, 0.5], [1.0; 3]],
        _ => {
            let first_span = 3 * length / 8;
            let blue_span = length - 2 * first_span;
            (0..length)
                .map(|index| {
                    let red = ((index + 1) as f64 / first_span as f64).min(1.0);
                    let green = if index < first_span {
                        0.0
                    } else {
                        ((index + 1 - first_span) as f64 / first_span as f64).min(1.0)
                    };
                    let blue = if index < 2 * first_span {
                        0.0
                    } else {
                        (index + 1 - 2 * first_span) as f64 / blue_span as f64
                    };
                    [red, green, blue]
                })
                .collect()
        }
    }
}

fn bone(length: usize) -> Vec<[f64; 3]> {
    gray(length)
        .into_iter()
        .zip(hot(length))
        .map(|(gray, hot)| {
            [
                (7.0 * gray[0] + hot[2]) / 8.0,
                (7.0 * gray[1] + hot[1]) / 8.0,
                (7.0 * gray[2] + hot[0]) / 8.0,
            ]
        })
        .collect()
}

fn copper(length: usize) -> Vec<[f64; 3]> {
    gray(length)
        .into_iter()
        .map(|value| {
            [
                (value[0] * 1.25).min(1.0),
                (value[1] * 0.7812).min(1.0),
                (value[2] * 0.4975).min(1.0),
            ]
        })
        .collect()
}

fn pink(length: usize) -> Vec<[f64; 3]> {
    gray(length)
        .into_iter()
        .zip(hot(length))
        .map(|(gray, hot)| {
            [
                ((2.0 * gray[0] + hot[0]) / 3.0).sqrt(),
                ((2.0 * gray[1] + hot[1]) / 3.0).sqrt(),
                ((2.0 * gray[2] + hot[2]) / 3.0).sqrt(),
            ]
        })
        .collect()
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn jet(length: usize) -> Vec<[f64; 3]> {
    match length {
        0 => Vec::new(),
        1 => vec![[0.0, 1.0, 1.0]],
        2 => vec![[0.0, 0.0, 1.0], [0.0, 1.0, 1.0]],
        _ => {
            let ramp_length = length.div_ceil(4);
            let mut first = (3 * length).div_ceil(8) + 1;
            if length % 8 == 2 {
                first += 1;
            }
            let second = first + ramp_length - 1;
            let third = (second + ramp_length).min(length);
            let mut colors = vec![[0.0; 3]; length];
            apply_jet_middle_channel(&mut colors, 0, first, second, third, ramp_length);

            first -= ramp_length;
            let second = first + ramp_length - 1;
            let third = (second + ramp_length).min(length);
            apply_jet_middle_channel(&mut colors, 1, first, second, third, ramp_length);

            let blue_first = second.saturating_sub(ramp_length).max(1);
            let leading_span = ramp_length.min(blue_first - 1);
            for one_based in 1..=blue_first {
                colors[one_based - 1][2] =
                    (ramp_length - leading_span + one_based - 1) as f64 / ramp_length as f64;
            }
            for one_based in blue_first..=second {
                colors[one_based - 1][2] = 1.0;
            }
            let trailing_span = ramp_length.min(length - third);
            for offset in 0..=trailing_span {
                colors[second + offset - 1][2] = (ramp_length - offset) as f64 / ramp_length as f64;
            }
            colors
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn apply_jet_middle_channel(
    colors: &mut [[f64; 3]],
    channel: usize,
    first: usize,
    second: usize,
    third: usize,
    ramp_length: usize,
) {
    for one_based in first..=second.min(colors.len()) {
        colors[one_based - 1][channel] = (one_based + 1 - first) as f64 / ramp_length as f64;
    }
    for one_based in (second + 1)..=third.min(colors.len()) {
        colors[one_based - 1][channel] = 1.0;
    }
    for one_based in (third + 1)..=(third + ramp_length).min(colors.len()) {
        colors[one_based - 1][channel] =
            (ramp_length + third - one_based) as f64 / ramp_length as f64;
    }
}

fn repeat_palette(length: usize, palette: &[[f64; 3]]) -> Vec<[f64; 3]> {
    (0..length)
        .map(|index| palette[index % palette.len()])
        .collect()
}

#[allow(clippy::cast_precision_loss, clippy::float_cmp)]
fn colorcube(length: usize) -> Vec<[f64; 3]> {
    if length < 8 {
        return gray(length);
    }
    if length == 8 {
        return vec![
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
        ];
    }

    let cube_length = integer_cube_root(length);
    let reserve = length - cube_length.pow(3);
    let blue_length = if reserve == 0 {
        cube_length - 1
    } else {
        cube_length
    };
    let red = unit_steps(cube_length);
    let green = unit_steps(cube_length);
    let blue = unit_steps(blue_length);
    let mut colors = Vec::with_capacity(length);
    for blue_value in blue {
        for red_value in &red {
            for green_value in &green {
                let color = [*red_value, *green_value, blue_value];
                let is_gray = color[0] == color[1] && color[1] == color[2];
                let zero_count = color.iter().filter(|component| **component == 0.0).count();
                if !is_gray && zero_count != 2 {
                    colors.push(color);
                }
            }
        }
    }

    let gradient_slots = length - colors.len() - 1;
    let color_steps = gradient_slots / 4;
    let gray_steps = gradient_slots - 3 * color_steps;
    colors.extend((1..=color_steps).map(|step| [step as f64 / color_steps as f64, 0.0, 0.0]));
    colors.extend((1..=color_steps).map(|step| [0.0, step as f64 / color_steps as f64, 0.0]));
    colors.extend((1..=color_steps).map(|step| [0.0, 0.0, step as f64 / color_steps as f64]));
    colors.push([0.0; 3]);
    colors.extend((1..=gray_steps).map(|step| {
        let value = step as f64 / gray_steps as f64;
        [value; 3]
    }));
    colors
}

fn integer_cube_root(value: usize) -> usize {
    let mut root = 0usize;
    while (root + 1).checked_pow(3).is_some_and(|cube| cube <= value) {
        root += 1;
    }
    root
}

#[allow(clippy::cast_precision_loss)]
fn unit_steps(length: usize) -> Vec<f64> {
    if length == 1 {
        vec![0.0]
    } else {
        (0..length)
            .map(|index| index as f64 / (length - 1) as f64)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_COLORMAP_LENGTH, PARULA_R2022B, PredefinedColormap, TURBO_R2022B, parula_r2022b,
        predefined_colormap_r2022b,
    };

    #[test]
    #[allow(clippy::float_cmp)]
    fn r2022b_base_table_has_exact_observed_endpoints() {
        assert_eq!(PARULA_R2022B.len(), DEFAULT_COLORMAP_LENGTH);
        assert_eq!(PARULA_R2022B[0], [0.2422, 0.1504, 0.6603]);
        assert_eq!(PARULA_R2022B[255], [0.9769, 0.9839, 0.0805]);
    }

    #[test]
    fn requested_lengths_resample_the_full_table_like_r2022b() {
        assert_eq!(parula_r2022b(0).len(), 0);
        assert_eq!(parula_r2022b(1), vec![[0.9769, 0.9839, 0.0805]]);
        assert_eq!(
            parula_r2022b(2),
            vec![[0.2422, 0.1504, 0.6603], [0.9769, 0.9839, 0.0805]]
        );
        assert_eq!(parula_r2022b(256), PARULA_R2022B);
        let five = parula_r2022b(5);
        assert_eq!(five[3][1].to_bits(), 0.757_274_999_999_999_9_f64.to_bits());
        assert_eq!(five[1][2].to_bits(), 0.991_1_f64.to_bits());
    }

    #[test]
    fn catalog_contains_every_r2022b_predefined_name() {
        assert_eq!(PredefinedColormap::ALL.len(), 20);
        for name in [
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
        ] {
            assert_eq!(PredefinedColormap::from_name(name).unwrap().name(), name);
            assert_eq!(
                PredefinedColormap::from_name(&name.to_uppercase())
                    .unwrap()
                    .name(),
                name
            );
        }
        assert!(PredefinedColormap::from_name("sky").is_none());
        assert!(!PredefinedColormap::Vga.supports_length());
    }

    #[test]
    fn table_and_formula_maps_keep_r2022b_boundaries() {
        assert_eq!(TURBO_R2022B.len(), DEFAULT_COLORMAP_LENGTH);
        assert_eq!(
            predefined_colormap_r2022b(PredefinedColormap::Turbo, 1),
            vec![TURBO_R2022B[255]]
        );
        assert_eq!(
            predefined_colormap_r2022b(PredefinedColormap::Hot, 2),
            vec![[1.0, 1.0, 0.5], [1.0, 1.0, 1.0]]
        );
        assert_eq!(
            predefined_colormap_r2022b(PredefinedColormap::Jet, 4),
            vec![
                [0.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 0.0],
                [1.0, 0.0, 0.0]
            ]
        );
        assert_eq!(
            predefined_colormap_r2022b(PredefinedColormap::Colorcube, 8),
            vec![
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, 0.0, 0.0],
                [1.0, 1.0, 1.0]
            ]
        );
        assert_eq!(
            predefined_colormap_r2022b(PredefinedColormap::Vga, 999).len(),
            16
        );
    }
}
