'use strict';
'require view';
'require rpc';
'require ui';

var callInitStatus = rpc.declare({
    object: 'luci',
    method: 'getInitStatus',
    params: ['name'],
    expect: { result: {} }
});

var callInitAction = rpc.declare({
    object: 'luci',
    method: 'setInitAction',
    params: ['name', 'action'],
    expect: { result: false }
});

return view.extend({
    load: function() {
        return callInitStatus('aivpn');
    },

    render: function(status) {
        var running = status && status.running;

        return E('div', { 'class': 'cbi-map' }, [
            E('h2', {}, _('AIVPN Status')),
            E('div', { 'class': 'cbi-section' }, [
                E('div', { 'class': 'table', 'style': 'width:100%' }, [
                    E('div', { 'class': 'tr' }, [
                        E('div', { 'class': 'td', 'style': 'width:30%' }, _('Service')),
                        E('div', { 'class': 'td' }, [
                            E('span', {
                                'class': running
                                    ? 'label-status label ok'
                                    : 'label-status label warning'
                            }, running ? _('Running') : _('Stopped'))
                        ])
                    ]),
                    E('div', { 'class': 'tr' }, [
                        E('div', { 'class': 'td' }, _('Tunnel Interface')),
                        E('div', { 'class': 'td' }, running ? 'aivpn0' : '—')
                    ])
                ]),
                E('div', {
                    'class': 'cbi-section-actions',
                    'style': 'margin-top:12px'
                }, [
                    E('button', {
                        'class': 'btn cbi-button cbi-button-action',
                        'disabled': running ? true : null,
                        'click': ui.createHandlerFn(this, function() {
                            return callInitAction('aivpn', 'start').then(function() {
                                window.location.reload();
                            });
                        })
                    }, _('Start')),
                    ' ',
                    E('button', {
                        'class': 'btn cbi-button cbi-button-negative',
                        'disabled': running ? null : true,
                        'click': ui.createHandlerFn(this, function() {
                            return callInitAction('aivpn', 'stop').then(function() {
                                window.location.reload();
                            });
                        })
                    }, _('Stop'))
                ])
            ])
        ]);
    },

    handleSave: null,
    handleSaveApply: null,
    handleReset: null
});
