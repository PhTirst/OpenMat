classdef (Abstract) OpenMatAbstractMiddle < OpenMatAbstractBase
    methods
        function value = scale(object, input)
            value = input * 2;
        end

        function value = invokeProtected(object, input)
            value = object.hidden(input);
        end
    end

    methods (Abstract, Access = protected)
        value = hidden(object, input)
    end
end
