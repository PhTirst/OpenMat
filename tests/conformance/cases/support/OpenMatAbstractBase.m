classdef (Abstract) OpenMatAbstractBase
    methods
        function value = invoke(object, input)
            value = object.scale(input) + object.offset(input);
        end
    end

    methods (Abstract)
        value = scale(object, input)
        value = offset(object, input)
    end
end
