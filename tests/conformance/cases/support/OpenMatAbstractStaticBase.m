classdef (Abstract) OpenMatAbstractStaticBase
    methods
        function value = invoke(object, input)
            value = object.transform(input);
        end
    end

    methods (Abstract)
        value = transform(object, input)
    end
end
