classdef OpenMatAbstractStaticLeaf < OpenMatAbstractStaticBase
    methods (Static)
        function value = transform(input)
            value = input + 5;
        end
    end
end
