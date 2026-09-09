figure_handle = figure();

[contour_matrix, contour_handle] = contour([1 3 5; 2 4 6], [4 2 4]);
contour_limits = get(gca(), 'CLim');
contour_class = strcmp(class(contour_handle), 'matlab.graphics.chart.primitive.Contour');

image_handle = imagesc([1 2; 3 4]);
image_limits = get(gca(), 'CLim');
image_direction = get(gca(), 'YDir');
image_class = strcmp(class(image_handle), 'matlab.graphics.primitive.Image');

openmat_result = [ ...
    contour_class, size(contour_matrix, 1), all(contour_limits == [2 4]), ...
    image_class, all(image_limits == [1 4]), strcmp(image_direction, 'reverse') ...
];

close(figure_handle);
