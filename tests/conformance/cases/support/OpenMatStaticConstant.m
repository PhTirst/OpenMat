classdef OpenMatStaticConstant
    properties (Constant)
        Factor = 3
    end

    methods (Static)
        function result = scale(value)
            result = OpenMatStaticConstant.Factor * value;
        end
    end
end
