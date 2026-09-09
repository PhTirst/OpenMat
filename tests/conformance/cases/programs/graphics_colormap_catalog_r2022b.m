figure_handle = figure();
colormap([0 0 0; 0.25 0.25 0.25; 0.5 0.5 0.5; 1 1 1]);

turbo_default = turbo();
jet_default = jet();

parula_map = parula(5);
turbo_map = turbo(5);
hsv_map = hsv(5);
hot_map = hot(5);
cool_map = cool(5);
spring_map = spring(5);
summer_map = summer(5);
autumn_map = autumn(5);
winter_map = winter(5);
gray_map = gray(5);
bone_map = bone(5);
copper_map = copper(5);
pink_map = pink(5);
jet_map = jet(5);
lines_map = lines(5);
colorcube_map = colorcube(8);
prism_map = prism(5);
flag_map = flag(5);
white_map = white(5);
vga_map = vga();

catalog_values = [ ...
    parula_map(:); turbo_map(:); hsv_map(:); hot_map(:); ...
    cool_map(:); spring_map(:); summer_map(:); autumn_map(:); ...
    winter_map(:); gray_map(:); bone_map(:); copper_map(:); ...
    pink_map(:); jet_map(:); lines_map(:); colorcube_map(:); ...
    prism_map(:); flag_map(:); white_map(:); vga_map(:) ...
];
catalog_values = round(catalog_values .* 1e12) ./ 1e12;

openmat_result = [ ...
    size(turbo_default), size(jet_default), size(vga_map), ...
    catalog_values.' ...
];
close(figure_handle);
