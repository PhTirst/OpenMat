% Run the experiment-control example, select the comparison Scope,
% then click Write to m workspace using the variable name simout.
% Columns: time, reference, simulation, synthetic observation.
t = simout(:,1);
y = simout(:,3);
y_measured = simout(:,4);
figure;
plot(t,y,t,y_measured);
xlabel('Time (s)');
ylabel('Response');
legend('Simulation','Synthetic observation');
title('PI feedback: simulation and observation');
rmse = sqrt(mean((y-y_measured).^2))
