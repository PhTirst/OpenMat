classdef Control < matlab.ui.componentcontainer.ComponentContainer
    properties
        Text = ''
        Value = ''
        Placeholder = ''
        FontSize = 13
        Min = 0
        Max = 100
        Step = 1
        Items = {}
        Columns = {}
        Data = {}
        Editable = false
        Source = ''
        Alt = ''
        Fit = 'contain'
        Variant = 'default'
        Orientation = 'horizontal'
        SelectedIndex = 1
        FigureIndex = 1
        Width = 840
        Height = 560
        Color = '#247951'
        ButtonPushedFcn = []
        ValueChangedFcn = []
        CellEditCallback = []
        SelectionChangedFcn = []
    end
    methods
        function obj = Control(type)
            if nargin > 0
                obj.Type = type;
            end
        end
    end
end
