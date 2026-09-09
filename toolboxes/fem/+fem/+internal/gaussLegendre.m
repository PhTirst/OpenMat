function [x, w] = gaussLegendre(n)
%GAUSSLEGENDRE Small Gaussian rules on [0,1], no external numerical backend.
    switch n
        case 1
            x = 0;
            w = 2;
        case 2
            x = [-1; 1] / sqrt(3);
            w = [1; 1];
        case 3
            x = [-sqrt(3/5); 0; sqrt(3/5)];
            w = [5; 8; 5] / 9;
        case 4
            a = sqrt((3 + 2*sqrt(6/5)) / 7);
            b = sqrt((3 - 2*sqrt(6/5)) / 7);
            x = [-a; -b; b; a];
            w = [18-sqrt(30); 18+sqrt(30); 18+sqrt(30); 18-sqrt(30)] / 36;
        case 5
            a = sqrt(5 + 2*sqrt(10/7)) / 3;
            b = sqrt(5 - 2*sqrt(10/7)) / 3;
            x = [-a; -b; 0; b; a];
            w = [(322-13*sqrt(70))/900; (322+13*sqrt(70))/900; 128/225; ...
                (322+13*sqrt(70))/900; (322-13*sqrt(70))/900];
        otherwise
            error('fem:UnsupportedQuadrature', 'Unsupported one-dimensional Gaussian rule.');
    end
    x = (x + 1) / 2;
    w = w / 2;
end
