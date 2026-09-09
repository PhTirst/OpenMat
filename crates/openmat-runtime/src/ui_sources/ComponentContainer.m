classdef ComponentContainer < handle
    properties
        Id = ''
        Name = ''
        Type = 'ComponentContainer'
        Title = ''
        Visible = true
        Enable = true
        Tooltip = ''
        Children = {}
        Position = []
        Layout = struct('Mode', 'column', 'X', 16, 'Y', 16, 'Width', 180, 'Height', 36, 'Row', 1, 'Column', 1, 'RowSpan', 1, 'ColumnSpan', 1, 'Columns', 2, 'Gap', 12, 'Padding', 12, 'Grow', 0)
    end
    properties (SetAccess = private)
        Parent = []
        RuntimeId = ''
    end
    properties (Access = private)
        Initialized = false
        Initializing = false
        Updating = false
        Disposing = false
        OwnedListeners = {}
    end
    methods
        function obj = ComponentContainer()
            obj.RuntimeId = openmat_ui('identity', obj);
        end
        function initialize(obj)
            if ~isvalid(obj) || obj.Initialized || obj.Initializing || obj.Disposing
                return;
            end
            obj.Initializing = true;
            try
                obj.buildDesignerComponents();
                obj.setup();
                children = obj.Children;
                for index = 1:numel(children)
                    if isvalid(children{index})
                        children{index}.initialize();
                    end
                end
                obj.Initialized = true;
                obj.Initializing = false;
                obj.refresh();
            catch problem
                obj.Initializing = false;
                rethrow(problem);
            end
        end
        function refresh(obj)
            if ~isvalid(obj) || obj.Updating || obj.Disposing
                return;
            end
            if ~obj.Initialized
                obj.initialize();
                return;
            end
            obj.Updating = true;
            try
                obj.update();
                children = obj.Children;
                for index = 1:numel(children)
                    if isvalid(children{index})
                        children{index}.refresh();
                    end
                end
                obj.Updating = false;
            catch problem
                obj.Updating = false;
                rethrow(problem);
            end
        end
        function add(obj, child)
            if ~isa(child, 'matlab.ui.componentcontainer.ComponentContainer') || ~isvalid(child)
                error('A UI child must be a live component.');
            end
            ancestor = obj;
            while ~isempty(ancestor)
                if strcmp(ancestor.RuntimeId, child.RuntimeId)
                    error('A UI component cannot contain itself or an ancestor.');
                end
                ancestor = ancestor.Parent;
            end
            if ~isempty(child.Parent) && isvalid(child.Parent)
                if strcmp(child.Parent.RuntimeId, obj.RuntimeId)
                    return;
                end
                child.Parent.remove(child);
            end
            children = obj.Children;
            children{numel(children) + 1} = child;
            obj.Children = children;
            child.Parent = obj;
            if obj.Initialized && ~obj.Initializing
                child.initialize();
            end
        end
        function remove(obj, child)
            kept = {};
            children = obj.Children;
            for index = 1:numel(children)
                current = children{index};
                if isvalid(current) && ~strcmp(current.RuntimeId, child.RuntimeId)
                    kept{numel(kept) + 1} = current;
                end
            end
            obj.Children = kept;
            if isvalid(child) && ~isempty(child.Parent) && isvalid(child.Parent) && strcmp(child.Parent.RuntimeId, obj.RuntimeId)
                child.Parent = [];
            end
        end
        function found = findRuntime(obj, key)
            found = [];
            if strcmp(obj.RuntimeId, key)
                found = obj;
                return;
            end
            children = obj.Children;
            for index = 1:numel(children)
                if isvalid(children{index})
                    found = children{index}.findRuntime(key);
                    if ~isempty(found)
                        return;
                    end
                end
            end
        end
        function listener = listen(obj, source, signal, callback)
            listener = addlistener(source, signal, callback);
            listeners = obj.OwnedListeners;
            listeners{numel(listeners) + 1} = listener;
            obj.OwnedListeners = listeners;
        end
        function delete(obj)
            if obj.Disposing
                return;
            end
            % A direct method call can enter before handle finalization.
            % Finalization itself permits only reads of the dying object's slots.
            live = isvalid(obj);
            if live
                obj.Disposing = true;
            end
            if ~isempty(obj.Parent) && isvalid(obj.Parent)
                obj.Parent.remove(obj);
            end
            children = obj.Children;
            listeners = obj.OwnedListeners;
            if live
                obj.Children = {};
                obj.OwnedListeners = {};
            end
            for index = 1:numel(listeners)
                if isvalid(listeners{index})
                    delete(listeners{index});
                end
            end
            for index = 1:numel(children)
                if isvalid(children{index})
                    delete(children{index});
                end
            end
            obj.teardown();
            openmat_ui('destroy', obj);
        end
    end
    methods (Access = protected)
        function buildDesignerComponents(obj)
        end
        function setup(obj)
        end
        function update(obj)
        end
        function teardown(obj)
        end
    end
end
